use crate::domain::ParamSetting;
use crate::infrastructure::api::convert::ConversionResult;
use realearn_api::schema::*;

pub fn convert_parameter(p: Parameter) -> ConversionResult<ParamSetting> {
    let data = ParamSetting {
        key: p.id,
        name: p.name.unwrap_or_default(),
        value_count: p.value_count,
        value_labels: p.value_labels.unwrap_or_default(),
    };
    Ok(data)
}
