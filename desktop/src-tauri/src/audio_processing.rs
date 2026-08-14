use crate::research_params::{AudioResearchParams, ParameterValidationError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioProcessingProfile {
    pub parameters_version: String,
    pub params: AudioResearchParams,
}

impl Default for AudioProcessingProfile {
    fn default() -> Self {
        Self {
            parameters_version: "audio_processing_v1".to_owned(),
            params: AudioResearchParams::default(),
        }
    }
}

impl AudioProcessingProfile {
    pub fn validate(&self) -> Result<(), Vec<ParameterValidationError>> {
        let mut errors = Vec::new();
        if self.parameters_version.trim().is_empty() || self.parameters_version.len() > 64 {
            errors.push(ParameterValidationError {
                field: "parameters_version".to_owned(),
                code: "invalid_parameters_version".to_owned(),
                unit: "版本".to_owned(),
                value: None,
                min: None,
                max: Some(64.0),
                message: "音频处理参数版本不能为空且不能超过 64 个字节".to_owned(),
            });
        }
        if let Err(mut params_errors) = self.params.validate() {
            errors.append(&mut params_errors);
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
