//! Plugin creation schemas and host-layer configuration validation.

use super::plugin::{PluginConfiguration, PluginConfigurationValue};
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, fmt::Display};
use thiserror::Error;
use toml::ser;

/// The semantic type of a plugin creation parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginParameterKind {
    /// Free-form text.
    String,
    /// A boolean value represented as `true` or `false`.
    Boolean,
    /// A signed integer value.
    Integer,
    /// A finite floating-point value.
    Float,
    /// A path to a file selected by the application and supplied through the host layer.
    File,
    /// A path to a directory selected by the application and supplied through the host layer.
    Directory,
    /// One value from [`PluginParameterSchema::choices`].
    Enum,
}

/// One field a plugin accepts when it creates a device.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginParameterSchema {
    /// Stable key used in [`PluginConfiguration`].
    pub name: String,
    /// Human-readable label suitable for an application UI.
    pub label: String,
    /// Semantic value type. This lets an application choose an appropriate editor.
    pub kind: PluginParameterKind,
    /// Whether the host layer requires the application to provide a value.
    #[serde(default)]
    pub required: bool,
    /// Optional explanatory text for an end user.
    #[serde(default)]
    pub description: Option<String>,
    /// Optional suggested default. It is not applied automatically.
    #[serde(default)]
    pub default: Option<String>,
    /// Optional file extensions for `File` parameters.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Valid values for an `Enum` parameter.
    #[serde(default)]
    pub choices: Vec<String>,
}

/// A plugin's versioned TOML creation schema.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PluginConfigurationSchema {
    /// Schema format version.
    pub version: u32,
    /// Fields accepted by the plugin.
    #[serde(default)]
    pub parameters: Vec<PluginParameterSchema>,
}

/// An error found while parsing a schema or validating a configuration.
#[derive(Debug, Error, PartialEq)]
pub enum PluginConfigurationError {
    /// The TOML schema could not be decoded.
    #[error("invalid plugin configuration schema: {0}")]
    InvalidSchema(String),
    /// The schema format is not supported by this host layer.
    #[error("unsupported plugin configuration schema version {0}")]
    UnsupportedVersion(u32),
    /// A schema contains a duplicate parameter name.
    #[error("duplicate plugin configuration parameter `{0}`")]
    DuplicateParameter(String),
    /// A required field was not supplied.
    #[error("required plugin configuration parameter `{0}` is missing")]
    MissingRequired(String),
    /// A configuration contains a field not declared by its schema.
    #[error("unknown plugin configuration parameter `{0}`")]
    UnknownParameter(String),
    /// A configuration contains the same field more than once.
    #[error("plugin configuration parameter `{0}` was supplied more than once")]
    DuplicateValue(String),
    /// A supplied value does not match the declared semantic type.
    #[error(
        "invalid value for plugin configuration parameter `{name}`; expected {expected}, got `{value}`"
    )]
    InvalidValue {
        name: String,
        expected: String,
        value: String,
    },
}

impl PluginConfigurationSchema {
    /// Returns an empty schema for legacy plugins that do not declare fields.
    pub fn empty() -> Self {
        Self {
            version: 1,
            parameters: Vec::new(),
        }
    }

    /// Parses and structurally validates a plugin's TOML schema.
    pub fn parse(source: &str) -> Result<Self, PluginConfigurationError> {
        let schema: Self = toml::from_str(source)
            .map_err(|error| PluginConfigurationError::InvalidSchema(error.to_string()))?;
        if schema.version != 1 {
            return Err(PluginConfigurationError::UnsupportedVersion(schema.version));
        }

        let mut names = HashSet::new();
        for parameter in &schema.parameters {
            if parameter.name.is_empty() || !names.insert(parameter.name.clone()) {
                return Err(PluginConfigurationError::DuplicateParameter(
                    parameter.name.clone(),
                ));
            }
            if parameter.kind == PluginParameterKind::Enum && parameter.choices.is_empty() {
                return Err(PluginConfigurationError::InvalidSchema(format!(
                    "enum parameter `{}` must declare at least one choice",
                    parameter.name
                )));
            }
        }
        Ok(schema)
    }

    /// Creates a configuration with one `None` entry for every schema field.
    pub fn new_configuration(&self, serial: &str) -> PluginConfiguration {
        PluginConfiguration {
            serial: serial.into(),
            values: self
                .parameters
                .iter()
                .map(|parameter| PluginConfigurationValue {
                    name: parameter.name.as_str().into(),
                    value: None.into(),
                })
                .collect(),
        }
    }

    /// Checks required fields, rejects unknown fields, and validates semantics.
    pub fn validate(
        &self,
        configuration: &PluginConfiguration,
    ) -> Result<(), PluginConfigurationError> {
        let mut supplied = HashSet::new();
        for value in configuration.values.iter() {
            let name = value.name.as_str();
            if !supplied.insert(name.to_owned()) {
                return Err(PluginConfigurationError::DuplicateValue(name.to_owned()));
            }
            let parameter = self
                .parameters
                .iter()
                .find(|parameter| parameter.name == name)
                .ok_or_else(|| PluginConfigurationError::UnknownParameter(name.to_owned()))?;
            if let Some(raw) = value.value.clone().into_option() {
                self.validate_value(parameter, raw.as_str())?;
            } else if parameter.required {
                return Err(PluginConfigurationError::MissingRequired(name.to_owned()));
            }
        }

        for parameter in &self.parameters {
            if parameter.required
                && !configuration
                    .values
                    .iter()
                    .any(|value| value.name.as_str() == parameter.name && value.value.is_some())
            {
                return Err(PluginConfigurationError::MissingRequired(
                    parameter.name.clone(),
                ));
            }
        }
        Ok(())
    }

    fn validate_value(
        &self,
        parameter: &PluginParameterSchema,
        value: &str,
    ) -> Result<(), PluginConfigurationError> {
        let valid = match parameter.kind {
            PluginParameterKind::String => true,
            PluginParameterKind::Boolean => value.parse::<bool>().is_ok(),
            PluginParameterKind::Integer => value.parse::<i64>().is_ok(),
            PluginParameterKind::Float => value.parse::<f64>().is_ok_and(f64::is_finite),
            PluginParameterKind::File | PluginParameterKind::Directory => !value.is_empty(),
            PluginParameterKind::Enum => parameter.choices.iter().any(|choice| choice == value),
        };
        if valid {
            Ok(())
        } else {
            Err(PluginConfigurationError::InvalidValue {
                name: parameter.name.clone(),
                expected: format!("{:?}", parameter.kind).to_lowercase(),
                value: value.to_owned(),
            })
        }
    }

    pub fn as_json(&self) -> serde_json::Result<String> {
        serde_json::to_string(self)
    }
}

impl Display for PluginConfigurationSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match ser::to_string(self) {
            Ok(res) => write!(f, "{}", res),
            Err(err) => write!(f, "{}{}", "[result]\nerror=", err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCHEMA: &str = r#"
version = 1

[[parameters]]
name = "input_file"
label = "Input file"
kind = "file"
required = true
extensions = ["raw"]
"#;

    #[test]
    fn schema_builds_all_optional_values() {
        let schema = PluginConfigurationSchema::parse(SCHEMA).unwrap();
        let config = schema.new_configuration("device-1");
        assert_eq!(config.serial, "device-1");
        assert!(config.values[0].value.is_none());
        assert_eq!(
            schema.validate(&config),
            Err(PluginConfigurationError::MissingRequired(
                "input_file".into()
            ))
        );
    }

    #[test]
    fn schema_accepts_a_supplied_file() {
        let schema = PluginConfigurationSchema::parse(SCHEMA).unwrap();
        let mut config = schema.new_configuration("device-1");
        config.values[0].value = Some("events.raw".into()).into();
        assert!(schema.validate(&config).is_ok());
    }
}
