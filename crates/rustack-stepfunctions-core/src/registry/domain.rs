//! Validated identities and bounded atomic tag mutations.

use std::{collections::BTreeMap, str::FromStr};

use regex::RegexBuilder;

use super::RegistryError;
use crate::TenantScope;

macro_rules! redacted {
    ($name:ident) => {
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "(<redacted>)"))
            }
        }
    };
}
pub(super) use redacted;
macro_rules! names {
    ($($name:ident),*) => {$(
        /// Validated ASCII workflow identifier, one to eighty bytes.
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        #[non_exhaustive]
        pub struct $name(String);
        redacted!($name);
        impl FromStr for $name {
            type Err=RegistryError;
            fn from_str(value:&str)->Result<Self,Self::Err> {
                if value.is_empty() || value.len()>80 || !value.bytes().all(|c| c.is_ascii_alphanumeric() || matches!(c,b'_'|b'-')) {return Err(RegistryError::Validation);}
                Ok(Self(value.into()))
            }
        }
        impl TryFrom<&str> for $name {
            type Error=RegistryError;
            fn try_from(value:&str)->Result<Self,Self::Error>{value.parse()}
        }
        impl $name {
            /// Read the validated identifier without diagnostic formatting.
            ///
            /// ```
            #[doc=concat!("let name:rustack_stepfunctions_core::",stringify!($name),"=\"example\".parse()?;\nassert_eq!(name.as_str(),\"example\");\n# Ok::<(),rustack_stepfunctions_core::RegistryError>(())")]
            /// ```
            #[must_use] pub fn as_str(&self)->&str {&self.0}
        }
    )*};
}
names!(StateMachineName, ExecutionName);

/// Whole, unqualified, scoped state machine ARN.
#[derive(Clone)]
#[non_exhaustive]
pub struct StateMachineArn {
    text: String,
    scope: TenantScope,
    name: StateMachineName,
}
redacted!(StateMachineArn);
impl StateMachineArn {
    /// Construct a whole ARN from validated operator scope and name.
    ///
    /// # Errors
    /// Rejects a resulting ARN above256 bytes.
    ///
    /// ```
    /// let arn=rustack_stepfunctions_core::StateMachineArn::new(&Default::default(),"example".parse()?)?;
    /// assert!(arn.as_str().ends_with(":stateMachine:example"));
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(scope: &TenantScope, name: StateMachineName) -> Result<Self, RegistryError> {
        let text = format!(
            "arn:{}:states:{}:{}:stateMachine:{}",
            scope.partition(),
            scope.region(),
            scope.account(),
            name.as_str()
        );
        if text.len() > 256 {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            text,
            scope: scope.clone(),
            name,
        })
    }
    /// Inspect the canonical complete ARN.
    ///
    /// ```
    /// # fn inspect(arn:&rustack_stepfunctions_core::StateMachineArn) { let text=arn.as_str(); }
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
    /// Inspect the validated name.
    ///
    /// ```
    /// # fn inspect(arn:&rustack_stepfunctions_core::StateMachineArn) { let name=arn.name(); }
    /// ```
    #[must_use]
    pub fn name(&self) -> &StateMachineName {
        &self.name
    }
    pub(super) fn check(&self, scope: &TenantScope) -> Result<(), RegistryError> {
        if same_scope(&self.scope, scope) {
            Ok(())
        } else {
            Err(RegistryError::Validation)
        }
    }
}
impl FromStr for StateMachineArn {
    type Err = RegistryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > 256 {
            return Err(RegistryError::Validation);
        }
        let mut parts = text.split(':');
        let (
            Some("arn"),
            Some(partition),
            Some("states"),
            Some(region),
            Some(account),
            Some("stateMachine"),
            Some(name),
            None,
        ) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        )
        else {
            return Err(RegistryError::Validation);
        };
        Self::new(
            &TenantScope::new(partition, region, account)?,
            name.parse()?,
        )
    }
}
/// Whole unqualified execution ARN, including its machine identity path.
#[derive(Clone)]
#[non_exhaustive]
pub struct ExecutionArn {
    text: String,
    machine: StateMachineArn,
    name: ExecutionName,
}
redacted!(ExecutionArn);
impl ExecutionArn {
    /// Construct a whole execution ARN.
    ///
    /// # Errors
    /// Rejects a resulting ARN above256 bytes.
    ///
    /// ```
    /// let machine="arn:aws:states:us-east-1:000000000000:stateMachine:example".parse()?;
    /// let execution=rustack_stepfunctions_core::ExecutionArn::new(machine,"run".parse()?)?;
    /// assert!(execution.as_str().ends_with(":execution:example:run"));
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(machine: StateMachineArn, name: ExecutionName) -> Result<Self, RegistryError> {
        let text = format!(
            "arn:{}:states:{}:{}:execution:{}:{}",
            machine.scope.partition(),
            machine.scope.region(),
            machine.scope.account(),
            machine.name.as_str(),
            name.as_str()
        );
        if text.len() > 256 {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            text,
            machine,
            name,
        })
    }
    /// Read the complete ARN.
    ///
    /// ```
    /// # fn inspect(arn:&rustack_stepfunctions_core::ExecutionArn) { let text=arn.as_str(); }
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
    /// Read the owning machine ARN.
    ///
    /// ```
    /// # fn inspect(arn:&rustack_stepfunctions_core::ExecutionArn) { let machine=arn.machine(); }
    /// ```
    #[must_use]
    pub fn machine(&self) -> &StateMachineArn {
        &self.machine
    }
    /// Read the execution name.
    ///
    /// ```
    /// # fn inspect(arn:&rustack_stepfunctions_core::ExecutionArn) { let name=arn.name(); }
    /// ```
    #[must_use]
    pub fn name(&self) -> &ExecutionName {
        &self.name
    }
}
impl FromStr for ExecutionArn {
    type Err = RegistryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > 256 {
            return Err(RegistryError::Validation);
        }
        let mut p = text.split(':');
        let (
            Some("arn"),
            Some(partition),
            Some("states"),
            Some(region),
            Some(account),
            Some("execution"),
            Some(machine),
            Some(name),
            None,
        ) = (
            p.next(),
            p.next(),
            p.next(),
            p.next(),
            p.next(),
            p.next(),
            p.next(),
            p.next(),
            p.next(),
        )
        else {
            return Err(RegistryError::Validation);
        };
        Self::new(
            StateMachineArn::new(
                &TenantScope::new(partition, region, account)?,
                machine.parse()?,
            )?,
            name.parse()?,
        )
    }
}
pub(super) fn same_scope(a: &TenantScope, b: &TenantScope) -> bool {
    a.partition() == b.partition() && a.region() == b.region() && a.account() == b.account()
}

/// Structurally validated IAM execution-role metadata; never an IAM grant.
#[derive(Clone)]
#[non_exhaustive]
pub struct RoleArn {
    text: String,
    partition: String,
    account: String,
}
redacted!(RoleArn);
impl FromStr for RoleArn {
    type Err = RegistryError;
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() > 256 {
            return Err(RegistryError::Validation);
        }
        let mut p = text.splitn(6, ':');
        let (Some("arn"), Some(partition), Some("iam"), Some(""), Some(account), Some(resource)) =
            (p.next(), p.next(), p.next(), p.next(), p.next(), p.next())
        else {
            return Err(RegistryError::Validation);
        };
        TenantScope::new(partition, "us-east-1", account)?;
        let role = resource
            .strip_prefix("role/")
            .ok_or(RegistryError::Validation)?;
        if role
            .split('/')
            .any(|part| part.is_empty() || !part.bytes().all(|b| matches!(b, 0x21..=0x7e)))
        {
            return Err(RegistryError::Validation);
        }
        let name = role.rsplit('/').next().ok_or(RegistryError::Validation)?;
        if name.len() > 64
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_+=,.@-".contains(&b))
        {
            return Err(RegistryError::Validation);
        }
        Ok(Self {
            text: text.into(),
            partition: partition.into(),
            account: account.into(),
        })
    }
}
impl RoleArn {
    /// Read validated metadata; logging remains the caller's trust boundary.
    ///
    /// ```
    /// let role:rustack_stepfunctions_core::RoleArn="arn:aws:iam::000000000000:role/example".parse()?;
    /// assert!(role.as_str().ends_with("role/example"));
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub(super) fn check(&self, scope: &TenantScope) -> Result<(), RegistryError> {
        if self.partition == scope.partition() && self.account == scope.account() {
            Ok(())
        } else {
            Err(RegistryError::Validation)
        }
    }
}
/// Validated, bounded, uniquely keyed tags; mutation is atomic.
#[derive(Clone, Default)]
#[non_exhaustive]
pub struct Tags(BTreeMap<String, String>);
redacted!(Tags);
impl Tags {
    /// Validate the entire mutation before constructing it.
    ///
    /// # Errors
    /// Rejects duplicate/reserved keys, invalid characters, byte/count limits.
    ///
    /// ```
    /// let tags=rustack_stepfunctions_core::Tags::new(vec![("team".into(),"engine".into())])?;
    /// assert_eq!(tags.values().len(),1);
    /// # Ok::<(),rustack_stepfunctions_core::RegistryError>(())
    /// ```
    pub fn new(values: Vec<(String, String)>) -> Result<Self, RegistryError> {
        if values.len() > 50 {
            return Err(RegistryError::Validation);
        }
        let regex = RegexBuilder::new(r"\A[\p{L}\p{Z}\p{N}_.:/=+@\-]*\z")
            .size_limit(1024 * 1024)
            .dfa_size_limit(1024 * 1024)
            .build()
            .map_err(RegistryError::Regex)?;
        let mut out = BTreeMap::new();
        for (key, value) in values {
            check_key(&key)?;
            if value.len() > 256
                || !regex.is_match(&key)
                || !regex.is_match(&value)
                || out.insert(key, value).is_some()
            {
                return Err(RegistryError::Validation);
            }
        }
        Ok(Self(out))
    }
    /// Read the bounded tag map.
    ///
    /// ```
    /// let tags=rustack_stepfunctions_core::Tags::default();
    /// assert!(tags.values().is_empty());
    /// ```
    #[must_use]
    pub fn values(&self) -> &BTreeMap<String, String> {
        &self.0
    }
    pub(super) fn merge(&self, other: &Self) -> Result<Self, RegistryError> {
        let mut out = self.0.clone();
        out.extend(other.0.clone());
        if out.len() > 50 {
            return Err(RegistryError::TagLimit);
        }
        Ok(Self(out))
    }
    pub(super) fn remove(&self, keys: &Self) -> Self {
        let mut out = self.0.clone();
        for key in keys.0.keys() {
            out.remove(key);
        }
        Self(out)
    }
}
fn check_key(key: &str) -> Result<(), RegistryError> {
    if key.is_empty()
        || key.len() > 128
        || key.get(..4).is_some_and(|v| v.eq_ignore_ascii_case("aws:"))
    {
        Err(RegistryError::Validation)
    } else {
        Ok(())
    }
}
