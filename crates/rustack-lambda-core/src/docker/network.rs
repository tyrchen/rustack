//! Read-only network qualification pins a bridge ID before allocation.

use bytes::Bytes;
use http::{Method, StatusCode};
use tokio::time::Instant;

use super::{
    ContainerPolicy, DockerError, DockerKey, DockerPort, NetworkSelection, Operation,
    check_deadline, decode, escaped, models::NetworkInspect,
};

/// Validated disconnected or full-ID local bridge selection; not startup readiness.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct QualifiedNetwork {
    requested: NetworkSelection,
    selected: NetworkSelection,
}
impl QualifiedNetwork {
    /// Bind the inspected network ID into a matching operator policy.
    ///
    /// # Errors
    /// Rejects a policy for a different originally requested/qualified network.
    ///
    /// ```no_run
    /// # fn bind(network:&rustack_lambda_core::docker::QualifiedNetwork, policy:&rustack_lambda_core::docker::ContainerPolicy) {
    /// let fixed=network.apply(policy);
    /// # }
    /// ```
    pub fn apply(&self, policy: &ContainerPolicy) -> Result<ContainerPolicy, DockerError> {
        if policy.network != self.requested && policy.network != self.selected {
            return Err(DockerError::Configuration);
        }
        let mut policy = policy.clone();
        policy.network = self.selected.clone();
        Ok(policy)
    }
    /// Whether the selected namespace deliberately has no external network.
    ///
    /// ```no_run
    /// # fn check(network:&rustack_lambda_core::docker::QualifiedNetwork) {
    /// let disconnected=network.disconnected();
    /// # }
    /// ```
    #[must_use]
    pub fn disconnected(&self) -> bool {
        self.selected.value() == "none"
    }
}
impl NetworkSelection {
    /// Inspect and pin an exact operator-owned local user bridge, or select none.
    ///
    /// No create/delete or host/default-network fallback occurs. A qualified
    /// bridge can still be deleted by the operator; allocation then fails.
    ///
    /// # Errors
    /// Rejects expired budgets, wire failure, incomplete/builtin/nonlocal network
    /// metadata, partial IDs or conflicting returned identity.
    ///
    /// ```no_run
    /// # async fn qualify(port:&rustack_lambda_core::docker::DockerPort)->Result<(),rustack_lambda_core::docker::DockerError>{
    /// let selection:rustack_lambda_core::docker::NetworkSelection="worker-network".parse()?;
    /// let network=selection.qualify(port,tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await?;
    /// # Ok(()) }
    /// ```
    pub async fn qualify(
        &self,
        port: &DockerPort,
        deadline: Instant,
    ) -> Result<QualifiedNetwork, DockerError> {
        check_deadline(deadline)?;
        let selected = if self.value() == "none" {
            self.clone()
        } else {
            let inspected = port
                .inspect_network(&self.value().parse()?, deadline)
                .await?;
            validate(self, &inspected)?
        };
        check_deadline(deadline)?;
        Ok(QualifiedNetwork {
            requested: self.clone(),
            selected,
        })
    }
}
impl DockerPort {
    /// Inspect bounded network metadata without mutating its lifetime.
    ///
    /// # Errors
    /// Returns wire admission, status, deadline or bounded metadata failure.
    ///
    /// ```no_run
    /// # async fn inspect(port:&rustack_lambda_core::docker::DockerPort,key:&rustack_lambda_core::docker::DockerKey){
    /// let network=port.inspect_network(key,tokio::time::Instant::now()+std::time::Duration::from_secs(5)).await;
    /// # }
    /// ```
    pub async fn inspect_network(
        &self,
        key: &DockerKey,
        deadline: Instant,
    ) -> Result<NetworkInspect, DockerError> {
        decode(
            self.call(
                Operation::Network,
                Method::GET,
                format!("/networks/{}", escaped(key.as_str())),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
            deadline,
        )
    }
}
fn validate(
    requested: &NetworkSelection,
    network: &NetworkInspect,
) -> Result<NetworkSelection, DockerError> {
    let name = network.name.as_deref().ok_or(DockerError::Configuration)?;
    name.parse::<DockerKey>()?;
    let id = network.id.as_deref().ok_or(DockerError::Configuration)?;
    let default = network
        .options
        .as_ref()
        .and_then(|options| options.get("com.docker.network.bridge.default_bridge"));
    if id.len() != 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !matches!(requested.value(),value if value==name||value==id)
        || matches!(name, "host" | "bridge" | "none" | "default")
        || network.driver.as_deref() != Some("bridge")
        || network.scope.as_deref() != Some("local")
        || network.ingress != Some(false)
        || network.config_only != Some(false)
        || default.is_some_and(|value| value != "false")
    {
        return Err(DockerError::Configuration);
    }
    id.parse()
}

#[cfg(test)]
mod tests;
