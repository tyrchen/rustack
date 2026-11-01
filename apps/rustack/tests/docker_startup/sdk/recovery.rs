//! Actual warm service witness across another installation's crash recovery.

use std::collections::BTreeSet;

use rustack_lambda_core::docker::{DockerWire, OwnerFilter};

use super::*;
use crate::process::{self, Server};

#[derive(Debug)]
pub(crate) struct Witness {
    client: Client,
    name: String,
    counter: u64,
}
impl Witness {
    pub(crate) async fn start(server: &Server) -> Result<Self> {
        Self::start_image(server, &process::handler()?).await
    }
    pub(crate) async fn start_image(server: &Server, image: &str) -> Result<Self> {
        let client = lambda(&server.endpoint);
        let name = format!("Survivor{}", Uuid::new_v4().simple());
        client
            .create_function()
            .function_name(&name)
            .role("arn:aws:iam::000000000000:role/Docker")
            .package_type(PackageType::Image)
            .architectures(if process::architecture()? == "arm64" {
                Architecture::Arm64
            } else {
                Architecture::X8664
            })
            .memory_size(128)
            .timeout(30)
            .code(FunctionCode::builder().image_uri(image).build())
            .send()
            .await?;
        active(&client, &name, "Active").await?;
        let mut witness = Self {
            client,
            name,
            counter: 0,
        };
        witness.advance().await?;
        Ok(witness)
    }
    pub(crate) async fn missing(&self) -> Result<()> {
        let error = self
            .client
            .get_function_configuration()
            .function_name(&self.name)
            .send()
            .await
            .err()
            .context("unsaved function survived failed shutdown")?;
        ensure!(
            error
                .as_service_error()
                .and_then(ProvideErrorMetadata::code)
                == Some("ResourceNotFoundException"),
            "wrong rejected unsaved function error"
        );
        Ok(())
    }
    pub(crate) async fn cold_revision(&mut self, expected: &str) -> Result<()> {
        let (value, error) = invoke(&self.client, &self.name, json!({"value":35})).await?;
        ensure!(
            error.is_none()
                && value["computed"] == 42
                && value["counter"] == 1
                && value["imageRevision"] == expected,
            "restore substituted the changed tag"
        );
        self.counter = 1;
        Ok(())
    }
    pub(crate) async fn advance(&mut self) -> Result<()> {
        let (value, error) = invoke(&self.client, &self.name, json!({"value":35})).await?;
        self.counter = self.counter.checked_add(1).context("witness counter")?;
        ensure!(
            error.is_none() && value["computed"] == 42 && value["counter"] == self.counter,
            "surviving installation lost its warm environment"
        );
        Ok(())
    }
}
pub(crate) async fn containers(server: &Server) -> Result<BTreeSet<String>> {
    let (owner, _) = server.observation_identity().await?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut wire = DockerWire::connect(process::socket()?.parse()?, deadline).await?;
    let result = wire
        .port()
        .list_containers(OwnerFilter::new(owner.parse()?, None, None)?, deadline)
        .await;
    let stopped = wire.shutdown().await;
    stopped?;
    result?
        .into_iter()
        .map(|row| row.id.context("observed full container ID"))
        .collect()
}
