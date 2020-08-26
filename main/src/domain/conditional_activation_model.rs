use crate::domain::ModifierCondition;
use derive_more::Display;
use enum_iterator::IntoEnumIterator;
use num_enum::{IntoPrimitive, TryFromPrimitive};
use serde::{Deserialize, Serialize};

#[derive(
    Copy,
    Clone,
    PartialEq,
    Debug,
    Serialize,
    Deserialize,
    IntoEnumIterator,
    TryFromPrimitive,
    IntoPrimitive,
    Display,
)]
#[repr(usize)]
pub enum ActivationType {
    #[serde(rename = "always")]
    #[display(fmt = "Always")]
    Always,
    #[serde(rename = "modifiers")]
    #[display(fmt = "When modifiers active")]
    Modifiers,
    #[serde(rename = "eel")]
    #[display(fmt = "EEL")]
    Eel,
}

#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize, Default)]
pub struct ModifierConditionModel {
    #[serde(rename = "paramIndex")]
    param_index: Option<u32>,
    #[serde(rename = "isOn")]
    is_on: bool,
}

impl ModifierConditionModel {
    pub fn create_modifier_condition(&self) -> Option<ModifierCondition> {
        self.param_index
            .map(|i| ModifierCondition::new(i, self.is_on))
    }

    pub fn param_index(&self) -> Option<u32> {
        self.param_index
    }

    pub fn with_param_index(&self, param_index: Option<u32>) -> ModifierConditionModel {
        ModifierConditionModel {
            param_index,
            ..*self
        }
    }

    pub fn is_on(&self) -> bool {
        self.is_on
    }

    pub fn with_is_on(&self, is_on: bool) -> ModifierConditionModel {
        ModifierConditionModel { is_on, ..*self }
    }
}