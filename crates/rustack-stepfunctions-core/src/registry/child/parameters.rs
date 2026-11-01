//! Validated optimized child-start parameters, preserving serialized Input bytes.

use serde_json::Value;

use super::{ChildError, ChildStartParameters};
use crate::{Payload, TenantScope};

impl ChildStartParameters {
    /// Validate the exact optimized fields before registry admission.
    ///
    /// # Errors
    /// Rejects unknown fields, invalid scope/name/JSON or excessive payload data.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{ChildStartParameters,Payload,TenantScope};
    /// let parameters=Payload::parse(br#"{"StateMachineArn":"arn:aws:states:us-east-1:000000000000:stateMachine:Child","Input":"null"}"#)?;
    /// let validated=ChildStartParameters::parse(&TenantScope::default(),&parameters)?;
    /// assert_eq!(validated.input().original_bytes(),b"null");
    /// # Ok::<(),Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(scope: &TenantScope, parameters: &Payload) -> Result<Self, ChildError> {
        let fields = parameters.value().as_object().ok_or(ChildError::Invalid)?;
        if fields.len() > 3
            || fields
                .keys()
                .any(|key| !matches!(key.as_str(), "StateMachineArn" | "Input" | "Name"))
        {
            return Err(ChildError::Invalid);
        }
        let machine = fields
            .get("StateMachineArn")
            .and_then(Value::as_str)
            .ok_or(ChildError::Invalid)?
            .parse()?;
        let name = fields
            .get("Name")
            .map(|value| {
                value
                    .as_str()
                    .ok_or(ChildError::Invalid)?
                    .parse()
                    .map_err(ChildError::from)
            })
            .transpose()?;
        let input = match fields.get("Input") {
            None | Some(Value::Null) => Payload::parse(b"{}")?,
            Some(Value::String(raw)) => Payload::parse(raw.as_bytes())?,
            Some(value) => Payload::from_value(value)?,
        };
        let result = Self {
            machine,
            name,
            input,
        };
        result.machine.check(scope)?;
        Ok(result)
    }
    /// Read the validated child input, including exact original serialized bytes.
    ///
    /// ```
    /// # fn inspect(p:&rustack_stepfunctions_core::ChildStartParameters){let input=p.input();}
    /// ```
    #[must_use]
    pub fn input(&self) -> &Payload {
        &self.input
    }
}
