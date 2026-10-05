use bevy_ecs::prelude::{Entity, World};

use crate::core::assets::AssetService;
use crate::data::dialogue::Id as DialogueId;
use crate::systems::item::{Inventory, ItemStack};
use crate::systems::reach::Tether;

pub trait Requirement: Send + Sync {
    fn met(&self, world: &World, player: Entity) -> bool;
    fn describe(&self) -> String;
}

pub trait Outcome: Send + Sync {
    fn apply(&self, ctx: &mut RuleContext);

    fn blocked(&self, _world: &World, _player: Entity) -> Option<String> {
        None
    }

    fn takes(&self) -> &[ItemStack] {
        &[]
    }

    fn gives(&self) -> &[ItemStack] {
        &[]
    }

    fn leads_to(&self) -> Vec<DialogueId> {
        Vec::new()
    }

    fn check(&self, _assets: &AssetService) {}
}

pub struct RuleContext<'w> {
    pub world: &'w mut World,
    pub player: Entity,
    pub encounter: Encounter,
    pub requires: &'w [&'static dyn Requirement],
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Encounter {
    pub with: Option<Entity>,
    pub tether: Option<Tether>,
}

pub struct Terms<'a> {
    pub requires: &'a [&'static dyn Requirement],
    pub costs: &'a [ItemStack],
    pub outcomes: &'a [&'a dyn Outcome],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuleRefusal(pub String);

pub struct Not(pub &'static dyn Requirement);

impl Requirement for Not {
    fn met(&self, world: &World, player: Entity) -> bool {
        !self.0.met(world, player)
    }

    fn describe(&self) -> String {
        format!("not {}", self.0.describe())
    }
}

pub struct AnyOf(pub &'static [&'static dyn Requirement]);

impl Requirement for AnyOf {
    fn met(&self, world: &World, player: Entity) -> bool {
        self.0
            .iter()
            .any(|requirement| requirement.met(world, player))
    }

    fn describe(&self) -> String {
        self.0
            .iter()
            .map(|requirement| requirement.describe())
            .collect::<Vec<_>>()
            .join(" or ")
    }
}

pub fn met(world: &World, player: Entity, requires: &[&dyn Requirement]) -> bool {
    requires
        .iter()
        .all(|requirement| requirement.met(world, player))
}

impl Terms<'_> {
    pub fn refusal(&self, world: &World, player: Entity) -> Option<RuleRefusal> {
        self.exchanged(world, player).err()
    }

    pub fn settle(
        &self,
        world: &mut World,
        player: Entity,
        encounter: Encounter,
    ) -> Result<(), RuleRefusal> {
        let exchanged = self.exchanged(world, player)?;
        if let Some(mut inventory) = world.get_mut::<Inventory>(player) {
            *inventory = exchanged;
        }
        let mut ctx = RuleContext {
            world,
            player,
            encounter,
            requires: self.requires,
        };
        for outcome in self.outcomes {
            outcome.apply(&mut ctx);
        }
        Ok(())
    }

    fn exchanged(&self, world: &World, player: Entity) -> Result<Inventory, RuleRefusal> {
        if let Some(unmet) = self
            .requires
            .iter()
            .find(|requirement| !requirement.met(world, player))
        {
            return Err(RuleRefusal(format!("Needs {}", unmet.describe())));
        }
        if let Some(reason) = self
            .outcomes
            .iter()
            .find_map(|outcome| outcome.blocked(world, player))
        {
            return Err(RuleRefusal(reason));
        }
        let takes: Vec<ItemStack> = self
            .costs
            .iter()
            .chain(self.outcomes.iter().flat_map(|outcome| outcome.takes()))
            .copied()
            .collect();
        let gives: Vec<ItemStack> = self
            .outcomes
            .iter()
            .flat_map(|outcome| outcome.gives())
            .copied()
            .collect();
        let mut inventory = world
            .get::<Inventory>(player)
            .cloned()
            .ok_or_else(|| RuleRefusal("Has no bag".to_owned()))?;
        inventory
            .exchange(&takes, &gives)
            .map_err(|refusal| RuleRefusal(refusal.describe()))?;
        Ok(inventory)
    }
}
