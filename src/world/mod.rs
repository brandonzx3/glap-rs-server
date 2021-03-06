use nalgebra::Vector2;
use nphysics2d::object::{RigidBody, Body, BodyPartHandle, DefaultColliderHandle};
use std::collections::{BTreeMap, BTreeSet};
use nphysics2d::force_generator::DefaultForceGeneratorSet;
use num_traits::Pow;
use nphysics2d::algebra::{Force2, ForceType, Inertia2};
use nphysics2d::joint::{DefaultJointConstraintHandle, MouseConstraint, JointConstraint};
use nphysics2d::math::Point;
use ncollide2d::pipeline::ContactEvent;
use crate::PartOfPlayer;
use generational_arena::{Arena, Index};
use crate::codec::ToClientMsg;
use std::ops::{Deref, DerefMut};

pub mod planets;
pub mod parts;
use parts::{Part, AttachedPartFacing, RecursivePartDescription};
use planets::AmPlanet;

pub mod nphysics_types {
    pub type MyUnits = f32;
    pub type MyHandle = generational_arena::Index;
    pub type MyIsometry = nphysics2d::math::Isometry<MyUnits>;
    pub type MyColliderHandle = nphysics2d::object::DefaultColliderHandle;
    pub type MyMechanicalWorld = nphysics2d::world::MechanicalWorld<MyUnits, MyHandle, MyColliderHandle>;
    pub type MyBodySet = super::World;
    pub type MyRigidBody = nphysics2d::object::RigidBody<MyUnits>;
    pub type MyGeometricalWorld = nphysics2d::world::GeometricalWorld<MyUnits, MyHandle, MyColliderHandle>;
    pub type MyColliderSet = nphysics2d::object::DefaultColliderSet<MyUnits, MyHandle>;
    pub type MyJointSet = nphysics2d::joint::DefaultJointConstraintSet<MyUnits, MyHandle>;
    pub type MyJointHandle = nphysics2d::joint::DefaultJointConstraintHandle;
    pub type MyForceSet = nphysics2d::force_generator::DefaultForceGeneratorSet<MyUnits, MyHandle>;
}
use nphysics_types::*;

pub struct Simulation {
    pub world: World,
    mechanics: MyMechanicalWorld,
    geometry: MyGeometricalWorld,
    pub colliders: MyColliderSet,
    pub joints: MyJointSet,
    persistant_forces: MyForceSet,
    pub planets: planets::Planets,
}
pub enum SimulationEvent {
    PlayerTouchPlanet { player: u16, part: MyHandle, planet: u16, },
    PlayerUntouchPlanet { player: u16, part: MyHandle, planet: u16 },
}


impl Simulation {
    pub fn new(step_time: f32) -> Simulation {
        let mut mechanics = MyMechanicalWorld::new(Vector2::new(0.0, 0.0));
        mechanics.set_timestep(step_time);
        mechanics.integration_parameters.max_ccd_substeps = 5;
        let geometry: MyGeometricalWorld = MyGeometricalWorld::new();
        let mut colliders: MyColliderSet = MyColliderSet::new();
        let mut bodies = World::default();
        let planets = planets::Planets::new(&mut colliders, &mut bodies);
        let simulation = Simulation {
            mechanics, geometry, colliders, world: bodies,
            joints: MyJointSet::new(),
            persistant_forces: MyForceSet::new(),
            planets,
        };
        simulation
    }

    fn celestial_gravity(&mut self) {
        for (_part_handle, part) in self.world.iter_parts_mut() {
            let part = part.body_mut();
            const GRAVITATION_CONSTANT: f32 = 1.0; //Lolrandom
            for body in &self.planets.celestial_objects() {
                let distance: (f32, f32) = ((body.position.0 - part.position().translation.x),
                                            (body.position.1 - part.position().translation.y));
                let magnitude: f32 = part.augmented_mass().linear * body.mass
                                     / (distance.0.pow(2f32) + distance.1.pow(2f32));
                                     //* GRAVITATION_CONSTANT;
                if distance.0.abs() > distance.1.abs() {
                    part.apply_force(0, &Force2::linear(Vector2::new(if distance.0 >= 0.0 { magnitude } else { -magnitude }, distance.1 / distance.0.abs() * magnitude)), ForceType::Force, false);
                } else {
                    part.apply_force(0, &Force2::linear(Vector2::new(distance.0 / distance.1.abs() * magnitude, if distance.1 >= 0.0 { magnitude } else { -magnitude })), ForceType::Force, false);
                }
            }
        }
    }

    pub fn simulate(&mut self, events: &mut Vec<SimulationEvent>) {
        self.celestial_gravity();
        self.mechanics.step(&mut self.geometry, &mut self.world, &mut self.colliders, &mut self.joints, &mut self.persistant_forces);
        for contact_event in self.geometry.contact_events() {
            match contact_event {
                ContactEvent::Started(handle1, handle2) => {
                    let planet: u16;
                    let other: DefaultColliderHandle;
                    if let Some(am_planet) = self.colliders.get(*handle1).unwrap().user_data().map(|any| any.downcast_ref::<AmPlanet>()).flatten() {
                        planet = am_planet.id; other = *handle2;
                    } else if let Some(am_planet) = self.colliders.get(*handle2).unwrap().user_data().map(|any| any.downcast_ref::<AmPlanet>()).flatten() {
                        planet = am_planet.id; other = *handle1;
                    } else { continue; }
                    let part_coll = self.colliders.get(other).unwrap();
                    if let Some(part) = self.world.get_part(part_coll.body()) {
                        if let Some(player_id) = part.part_of_player() {
                            events.push(SimulationEvent::PlayerTouchPlanet{ player: player_id, part: part_coll.body(), planet });
                        }
                    }
                },
                ContactEvent::Stopped(handle1, handle2) => {
                    let planet: u16;
                    let other: DefaultColliderHandle;
                    if let Some(am_planet) = self.colliders.get(*handle1).unwrap().user_data().map(|any| any.downcast_ref::<AmPlanet>()).flatten() {
                        planet = am_planet.id; other = *handle2;
                    } else if let Some(am_planet) = self.colliders.get(*handle2).unwrap().user_data().map(|any| any.downcast_ref::<AmPlanet>()).flatten() {
                        planet = am_planet.id; other = *handle1;
                    } else { continue; }
                    let part_coll = self.colliders.get(other).unwrap();
                    if let Some(part) = self.world.get_part(part_coll.body()) {
                        if let Some(player_id) = part.part_of_player() {
                            events.push(SimulationEvent::PlayerUntouchPlanet{ player: player_id, part: part_coll.body(), planet });
                        }
                    }
                }
            }
        }
    }

    pub fn equip_mouse_dragging(&mut self, part: MyHandle) -> DefaultJointConstraintHandle {
        let body = self.world.get_rigid_mut(part).unwrap();
        body.set_local_inertia(Inertia2::new(0.00000001, body.augmented_mass().angular));
        let space = body.position().translation;
        let constraint = MouseConstraint::new(
            BodyPartHandle(part, 0),
            BodyPartHandle(self.world.reference_point_body, 0),
            Point::new(0.0,0.0),
            Point::new(space.x, space.y),
            1000.0
        );
        self.joints.insert(constraint)
    }
    pub fn move_mouse_constraint(&mut self, constraint_id: DefaultJointConstraintHandle, x: f32, y: f32) {
        if let Some(Some(constraint)) = self.joints.get_mut(constraint_id).map(|c: &mut dyn JointConstraint<MyUnits, MyHandle>| c.downcast_mut::<MouseConstraint<MyUnits, MyHandle>>() ) {
            constraint.set_anchor_2(Point::new(x, y));
        }
    }
    pub fn release_constraint(&mut self, constraint_id: DefaultJointConstraintHandle) {
        self.joints.remove(constraint_id);
    }

    pub fn is_constraint_broken(&self, handle: DefaultJointConstraintHandle) -> bool {
        self.joints.get(handle).map(|joint| joint.is_broken()).unwrap_or(true)
    }

    pub fn geometrical_world(&self) -> &MyGeometricalWorld { &self.geometry }

    pub fn inflate(&mut self, parts: &RecursivePartDescription, initial_location: MyIsometry) -> MyHandle {
        parts.inflate(&mut (&mut self.world).into(), &mut self.colliders, &mut self.joints, initial_location)
    }
    pub fn delete_parts_recursive(&mut self, index: MyHandle) -> Vec<ToClientMsg> {
        let mut removal_msgs = Vec::new();
        self.world.delete_parts_recursive(index, &mut self.colliders, &mut self.joints, &mut removal_msgs);
        removal_msgs
    }
}

type MyStorage = Arena<WorldlyObject>;
pub struct World {
    storage: MyStorage,
    removal_events: std::collections::VecDeque<MyHandle>,
    reference_point_body: Index,
}

pub enum WorldlyObject {
    CelestialObject(MyRigidBody),
    Part(Part),
    Uninitialized,
}
impl WorldlyObject {
    pub fn rigid(&self) -> Option<&MyRigidBody> {
         match self {
            WorldlyObject::Part(part) => Some(part.body()),
            WorldlyObject::CelestialObject(body) => Some(body),
            WorldlyObject::Uninitialized => None
        }
    }
    pub fn rigid_mut(&mut self) -> Option<&mut MyRigidBody> {
        match self {
            WorldlyObject::Part(part) => Some(part.body_mut()),
            WorldlyObject::CelestialObject(body) => Some(body),
            WorldlyObject::Uninitialized => None
        }
    }

}

pub struct WorldAddHandle<'a>(&'a mut World); 
impl<'a> WorldAddHandle<'a> {
    pub fn add_now(&mut self, object: WorldlyObject) -> Index { self.0.storage.insert(object) }
    pub fn add_later(&mut self) -> Index { self.0.storage.insert(WorldlyObject::Uninitialized) }
    pub fn add_its_later(&mut self, index: Index, object: WorldlyObject) {
        match std::mem::replace(self.0.storage.get_mut(index).expect("add_its_later: the index doesn't exist"), object) {
            WorldlyObject::Uninitialized => {},
            _ => panic!("add_its_later: the index wasn't WorldlyObject::Uninitialized. Storage is now poisioned(?)")
        }
    }
    pub fn deconstruct(self) -> &'a mut World { self.0 }
}
impl<'a> From<&'a mut World> for WorldAddHandle<'a> {
    fn from(world: &'a mut World) -> WorldAddHandle<'a> { WorldAddHandle(world) }
}

impl World {
    pub fn get_rigid(&self, index: MyHandle) -> Option<&MyRigidBody> {
        self.storage.get(index).map(|obj| obj.rigid()).flatten()
    }
    pub fn get_part(&self, index: MyHandle) -> Option<&Part> {
        self.storage.get(index).map(|obj| match obj { WorldlyObject::Part(part) => Some(part), _ => None }).flatten()
    }
    pub fn get_rigid_mut(&mut self, index: MyHandle) -> Option<&mut MyRigidBody> {
        self.storage.get_mut(index).map(|obj| obj.rigid_mut()).flatten()
    }
    pub fn get_part_mut(&mut self, index: MyHandle) -> Option<&mut Part> {
        self.storage.get_mut(index).map(|obj| match obj { WorldlyObject::Part(part) => Some(part), _ => None }).flatten()
    }
    pub fn delete_parts_recursive(&mut self, index: MyHandle, colliders: &mut MyColliderSet, joints: &mut MyJointSet, removal_msgs: &mut Vec<ToClientMsg>) {
        match self.storage.remove(index) {
            Some(WorldlyObject::Part(part)) => {
                self.removal_events.push_back(index);
                part.delete_recursive(self, colliders, joints, removal_msgs);
            },
            None => (),
            _ => panic!("Delete part called on non-part")
        }
    }
    pub fn add_celestial_object(&mut self, body: MyRigidBody) -> MyHandle { self.storage.insert(WorldlyObject::CelestialObject(body)) }

    pub fn recurse_part<'a, F>(&'a self, part_handle: MyHandle, details: PartVisitDetails, func: &mut F)
    where F: FnMut(PartVisitHandle<'a>) {
        if let Some(part) = self.get_part(part_handle) {
            func(PartVisitHandle(self, part_handle, part, details));
            let attachment_dat = part.kind().attachment_locations();
            for (i, attachment) in part.attachments().iter().enumerate() {
                if let (Some(attachment), Some(attachment_dat)) = (attachment, attachment_dat[i]) {
                    let true_facing = attachment_dat.facing.compute_true_facing(details.true_facing);
                    let delta_rel_part = true_facing.delta_rel_part();
                    self.recurse_part(**attachment, PartVisitDetails {
                        part_rel_x: details.part_rel_x + delta_rel_part.0,
                        part_rel_y: details.part_rel_y + delta_rel_part.1,
                        my_facing: attachment_dat.facing,
                        true_facing
                    }, func);
                }
            }
        }
    }
    pub fn recurse_part_mut<'a, F>(&'a mut self, part_handle: MyHandle, details: PartVisitDetails, func: &mut F)
    where F: FnMut(PartVisitHandleMut<'_>) {
        if self.get_part_mut(part_handle).is_some() {
            func(PartVisitHandleMut(self, part_handle, details));
            let part = self.get_part(part_handle).unwrap();
            let attachment_dat = part.kind().attachment_locations();
            for (i, attachment) in part.attachments().iter().map(|attachment| attachment.as_ref().map(|attach| **attach)).collect::<Vec<_>>().into_iter().enumerate() {
                if let (Some(attachment), Some(attachment_dat)) = (attachment, attachment_dat[i]) {
                    let true_facing = attachment_dat.facing.compute_true_facing(details.true_facing);
                    let delta_rel_part = true_facing.delta_rel_part();
                    let details = PartVisitDetails {
                        part_rel_x: details.part_rel_x + delta_rel_part.0,
                        part_rel_y: details.part_rel_y + delta_rel_part.1,
                        my_facing: attachment_dat.facing,
                        true_facing
                    };
                    self.recurse_part_mut(attachment, details, func);
                }
            }
        }
    }
    pub fn recurse_part_with_return<'a, V, F>(&'a self, part_handle: MyHandle, details: PartVisitDetails, func: &mut F) -> Option<V>
    where F: FnMut(PartVisitHandle<'a>) -> Option<V> {
        if let Some(part) = self.get_part(part_handle) {
            let result = func(PartVisitHandle(self, part_handle, part, details));
            if result.is_some() { return result };
            let attachment_dat = part.kind().attachment_locations();
            for (i, attachment) in part.attachments().iter().enumerate() {
                if let (Some(attachment), Some(attachment_dat)) = (attachment, attachment_dat[i]) {
                    let true_facing = attachment_dat.facing.compute_true_facing(details.true_facing);
                    let delta_rel_part = true_facing.delta_rel_part();
                    if let Some(result) = self.recurse_part_with_return(**attachment, PartVisitDetails {
                        part_rel_x: details.part_rel_x + delta_rel_part.0,
                        part_rel_y: details.part_rel_y + delta_rel_part.1,
                        my_facing: attachment_dat.facing,
                        true_facing
                    }, func) {
                        return Some(result)
                    }
                }
            }
        }
        return None;
    }
    pub fn recurse_part_mut_with_return<'a, V, F>(&'a mut self, part_handle: MyHandle, details: PartVisitDetails, func: &mut F) -> Option<V>
    where F: FnMut(PartVisitHandleMut<'_>) -> Option<V> {
        if self.get_part_mut(part_handle).is_some() {
            let result = func(PartVisitHandleMut(self, part_handle, details));
            if result.is_some() { return result };
            drop(result);
            let part = self.get_part_mut(part_handle).unwrap();
            let attachment_dat = part.kind().attachment_locations();
            for (i, attachment) in part.attachments().iter().map(|attachment| attachment.as_ref().map(|attach| **attach)).collect::<Vec<_>>().into_iter().enumerate() {
                if let (Some(attachment), Some(attachment_dat)) = (attachment, attachment_dat[i]) {
                    let true_facing = attachment_dat.facing.compute_true_facing(details.true_facing);
                    let delta_rel_part = true_facing.delta_rel_part();
                    if let Some(result) = self.recurse_part_mut_with_return(attachment, PartVisitDetails {
                        part_rel_x: details.part_rel_x + delta_rel_part.0,
                        part_rel_y: details.part_rel_y + delta_rel_part.1,
                        my_facing: attachment_dat.facing,
                        true_facing
                    }, func) {
                        return Some(result)
                    }
                }
            }
        }
        return None;
    }

    pub fn recursive_detach_one(&mut self, parent_handle: MyHandle, attachment_slot: usize, player: &mut Option<&mut crate::PlayerMeta>, joints: &mut MyJointSet, parts_affected: &mut BTreeSet<MyHandle>) {
        if let Some(parent) = self.get_part_mut(parent_handle) {
            if let Some(attachment_handle) = parent.detach_part_player_agnostic(attachment_slot, joints) {
                parts_affected.insert(attachment_handle);
                if let Some(player) = player {
                    if let Some(attached_part) = self.get_part_mut(attachment_handle) {
                        attached_part.remove_from(*player);
                    }
                }
                self.recursive_detach_all(attachment_handle, player, joints, parts_affected);                                
            }
        }
    }
    pub fn recursive_detach_all(&mut self, parent_handle: MyHandle, player: &mut Option<&mut crate::PlayerMeta>, joints: &mut MyJointSet, parts_affected: &mut BTreeSet<MyHandle>) {
        if let Some(part) = self.get_part_mut(parent_handle) {
            for i in 0..part.attachments().len() {
                self.recursive_detach_one(parent_handle, i, player, joints, parts_affected);
            }
        }
    }
    pub fn remove_part_unprotected(&mut self, part_handle: MyHandle) -> Part {
        if let Some(WorldlyObject::Part(part)) = self.storage.remove(part_handle) {
            self.removal_events.push_back(part_handle);
            part
        } else { panic!("remove_part_unprotected") }
    }

    pub fn iter_parts_mut<'a>(&'a mut self) -> Box<dyn Iterator<Item=(MyHandle, &'a mut Part)> + 'a> {
        Box::new(self.storage.iter_mut().filter_map(|(handle, obj)| if let WorldlyObject::Part(part) = obj { Some((handle, part)) } else { None }))
    }
}
impl Default for World {
    fn default() -> World { 
        let mut storage = Arena::new();
        let reference_point_body = nphysics2d::object::RigidBodyDesc::new().status(nphysics2d::object::BodyStatus::Static).mass(0f32).build();
        let reference_point_body = storage.insert(WorldlyObject::CelestialObject(reference_point_body));
        World {
            storage,
            reference_point_body,
            removal_events: std::collections::VecDeque::<MyHandle>::new(),
        }
    }
}

impl nphysics2d::object::BodySet<MyUnits> for World {
    type Handle = MyHandle;
    fn get(&self, handle: Self::Handle) -> Option<&dyn nphysics2d::object::Body<MyUnits>> {
        if let Some(ptr) = self.get_rigid(handle) { Some(ptr) }
        else { None }
    }
    fn get_mut(&mut self, handle: Self::Handle) -> Option<&mut dyn nphysics2d::object::Body<MyUnits>> {
        if let Some(ptr) = self.get_rigid_mut(handle) { Some(ptr) }
        else { None }
    }
    fn contains(&self, handle: Self::Handle) -> bool {
        self.get_rigid(handle).is_some()
    }
    fn foreach(&self, f: &mut dyn FnMut(Self::Handle, &dyn nphysics2d::object::Body<MyUnits>)) {
        for (id, obj) in &self.storage {
            if let Some(body) = obj.rigid() { f(id, body) }
        }
    }
    fn foreach_mut(&mut self, f: &mut dyn FnMut(Self::Handle, &mut dyn nphysics2d::object::Body<MyUnits>)) {
        for (id, obj) in &mut self.storage {
            if let Some(body) = obj.rigid_mut() { f(id, body) }
        }
    }
    fn pop_removal_event(&mut self) -> Option<Self::Handle> {
        self.removal_events.pop_front()
    }
}

#[derive(Copy, Clone)]
pub struct PartVisitDetails {
    pub part_rel_x: i32,
    pub part_rel_y: i32,
    pub my_facing: AttachedPartFacing,
    pub true_facing: AttachedPartFacing,
}
impl Default for PartVisitDetails {
    fn default() -> Self { PartVisitDetails {
        part_rel_x: 0,
        part_rel_y: 0,
        my_facing: AttachedPartFacing::Up,
        true_facing: AttachedPartFacing::Up,
    } }
}

pub struct PartVisitHandle<'a> (&'a World, MyHandle, &'a Part, PartVisitDetails);
impl<'a> PartVisitHandle<'a> {
    pub fn get_part(&self, handle: MyHandle) -> Option<&Part> { self.0.get_part(handle) }
    pub fn get_rigid(&self, handle: MyHandle) -> Option<&MyRigidBody> { self.0.get_rigid(handle) }
    pub fn handle(&self) -> MyHandle { self.1 }
    pub fn details(&self) -> &PartVisitDetails { &self.3 }
}
impl<'a> Deref for PartVisitHandle<'a> {
    type Target = Part;
    fn deref(&self) -> &Part { self.2 }
}
pub struct PartVisitHandleMut<'a> (&'a mut World, MyHandle, PartVisitDetails);
impl<'a> PartVisitHandleMut<'a> {
    pub fn get_part(&self, handle: MyHandle) -> Option<&Part> { self.0.get_part(handle) }
    pub fn get_rigid(&self, handle: MyHandle) -> Option<&MyRigidBody> { self.0.get_rigid(handle) }
    pub fn get_part_mut(&mut self, handle: MyHandle) -> Option<&mut Part> { self.0.get_part_mut(handle) }
    pub fn get_rigid_mut(&mut self, handle: MyHandle) -> Option<&mut MyRigidBody> { self.0.get_rigid_mut(handle) }
    pub fn handle(&self) -> MyHandle { self.1 }
    pub fn details(&self) -> &PartVisitDetails { &self.2 }
}
impl<'a> Deref for PartVisitHandleMut<'a> {
    type Target = Part;
    fn deref(&self) -> &Part { self.get_part(self.1).unwrap() }
}
impl<'a> DerefMut for PartVisitHandleMut<'a> {
    fn deref_mut(&mut self) -> &mut Part { self.get_part_mut(self.1).unwrap() }
}
