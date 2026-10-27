//! Bounded, side-effect-free Image metadata validation before pin preparation.

pub(super) const IMAGE_METADATA_BYTES: usize = 16 * 1024 * 1024;
pub(super) const MAX_IMAGE_FUNCTIONS: usize = 256;
pub(super) const MAX_IMAGE_REVISIONS: usize = 1024;

#[cfg(unix)]
mod validation {
    use std::{
        io::{Error as IoError, ErrorKind, Result as IoResult, Write},
        iter::once,
    };

    use rustack_core::json::{JsonLimits, validate};
    use serde::Serialize;
    use serde_json::Value;
    use tokio::{
        sync::Semaphore,
        task::spawn_blocking,
        time::{Instant, timeout_at},
    };

    use super::{IMAGE_METADATA_BYTES, MAX_IMAGE_FUNCTIONS, MAX_IMAGE_REVISIONS};
    use crate::{
        docker::DockerError,
        error::LambdaServiceError,
        provider::{LambdaSnapshot, RustackLambda},
        resolver::{FunctionName, function_arn},
        storage::{
            FunctionRecord, FunctionSnapshotRef, LayerRecord, LayerSnapshotRef, MappingSnapshotRef,
            PolicyDocument, check_restore_deadline, image_error,
        },
    };

    static VALIDATORS: Semaphore = Semaphore::const_new(4);

    fn invalid() -> LambdaServiceError {
        LambdaServiceError::InvalidParameter {
            message: "Invalid or oversized Docker image archive metadata".into(),
        }
    }
    #[derive(Debug, Default)]
    struct BoundedBytes(Vec<u8>);
    impl Write for BoundedBytes {
        fn write(&mut self, bytes: &[u8]) -> IoResult<usize> {
            if bytes.len() > IMAGE_METADATA_BYTES.saturating_sub(self.0.len()) {
                return Err(IoError::from(ErrorKind::FileTooLarge));
            }
            self.0
                .try_reserve(bytes.len())
                .map_err(|_| IoError::from(ErrorKind::OutOfMemory))?;
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }
    #[derive(Debug)]
    struct GuardWork {
        remaining: usize,
        #[cfg(test)]
        visited_nodes: usize,
    }
    impl GuardWork {
        fn new() -> Self {
            Self {
                remaining: 65_536,
                #[cfg(test)]
                visited_nodes: 0,
            }
        }
        fn charge(&mut self, units: usize) -> Result<(), LambdaServiceError> {
            self.remaining = self.remaining.checked_sub(units).ok_or_else(invalid)?;
            Ok(())
        }
    }
    fn condition(value: &Value, work: &mut GuardWork) -> Result<(), LambdaServiceError> {
        let mut pending = vec![(value, 1usize)];
        while let Some((value, depth)) = pending.pop() {
            work.charge(1)?;
            #[cfg(test)]
            {
                work.visited_nodes = work.visited_nodes.checked_add(1).ok_or_else(invalid)?;
            }
            if depth > 64 {
                return Err(invalid());
            }
            let children = match value {
                Value::Array(values) => values.len(),
                Value::Object(values) => values.len(),
                Value::String(value) if value.len() > 128 * 1024 => return Err(invalid()),
                _ => 0,
            };
            if children > work.remaining.saturating_sub(pending.len()) {
                return Err(invalid());
            }
            let next_depth = depth.checked_add(1).ok_or_else(invalid)?;
            match value {
                Value::Array(values) => {
                    pending.extend(values.iter().map(|value| (value, next_depth)));
                }
                Value::Object(values) => {
                    for (key, value) in values {
                        if key.len() > 1024 {
                            return Err(invalid());
                        }
                        pending.push((value, next_depth));
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
    fn policy(policy: &PolicyDocument, work: &mut GuardWork) -> Result<(), LambdaServiceError> {
        if policy.statements.len() > 128 {
            return Err(invalid());
        }
        work.charge(policy.statements.len())?;
        for statement in &policy.statements {
            if let Some(value) = &statement.condition {
                condition(value, work)?;
            }
        }
        Ok(())
    }
    fn function_record(record: &FunctionRecord) -> Result<(), LambdaServiceError> {
        let mut work = GuardWork::new();
        work.charge(record.versions.len().checked_add(1).ok_or_else(invalid)?)?;
        policy(&record.policy, &mut work)?;
        for version in once(&record.latest).chain(record.versions.values()) {
            if version.package_type != "Image" || version.zip_bytes.is_some() {
                return Err(invalid());
            }
        }
        Ok(())
    }
    fn layer_record(record: &LayerRecord) -> Result<(), LambdaServiceError> {
        layer_record_guarded(record, &mut GuardWork::new())
    }
    fn layer_record_guarded(
        record: &LayerRecord,
        work: &mut GuardWork,
    ) -> Result<(), LambdaServiceError> {
        work.charge(record.versions.len())?;
        for version in record.versions.values() {
            policy(&version.policy, work)?;
        }
        Ok(())
    }
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct BorrowedSnapshot<'a> {
        functions: FunctionSnapshotRef<'a>,
        layers: LayerSnapshotRef<'a>,
        event_source_mappings: MappingSnapshotRef<'a>,
    }
    pub(crate) async fn export_image(
        provider: &RustackLambda,
        deadline: Instant,
    ) -> Result<LambdaSnapshot, LambdaServiceError> {
        check_restore_deadline(deadline)?;
        let permit = timeout_at(deadline, VALIDATORS.acquire())
            .await
            .map_err(|_| image_error(DockerError::Deadline))?
            .map_err(|_| image_error(DockerError::Stopped))?;
        let stores = (
            provider.store.clone(),
            provider.layer_store.clone(),
            provider.esm_store.clone(),
        );
        let scope = (
            provider.config.default_region.clone(),
            provider.config.account_id.clone(),
        );
        let mut job = spawn_blocking(move || {
            let retained = (stores, permit);
            check_restore_deadline(deadline)?;
            retained
                .0
                .0
                .check_snapshot_counts(MAX_IMAGE_FUNCTIONS, MAX_IMAGE_REVISIONS)?;
            let borrowed = BorrowedSnapshot {
                functions: retained.0.0.borrow_snapshot(function_record),
                layers: retained.0.1.borrow_snapshot(layer_record),
                event_source_mappings: retained.0.2.borrow_snapshot(),
            };
            let mut bytes = BoundedBytes::default();
            serde_json::to_writer(&mut bytes, &borrowed).map_err(|_| invalid())?;
            let limits = JsonLimits::new(IMAGE_METADATA_BYTES, 64, 65_536, 128 * 1024, 1024)
                .map_err(|_| invalid())?;
            validate(&bytes.0, limits).map_err(|_| invalid())?;
            check_restore_deadline(deadline)?;
            let mut snapshot: LambdaSnapshot =
                serde_json::from_slice(&bytes.0).map_err(|_| invalid())?;
            input(&snapshot, &scope.0, &scope.1)?;
            snapshot
                .functions
                .functions
                .sort_by(|left, right| left.name.cmp(&right.name));
            snapshot
                .layers
                .layers
                .sort_by(|left, right| left.name.cmp(&right.name));
            snapshot
                .event_source_mappings
                .mappings
                .sort_by(|left, right| left.uuid.cmp(&right.uuid));
            check_restore_deadline(deadline)?;
            Ok(snapshot)
        });
        let result = timeout_at(deadline, &mut job)
            .await
            .map_err(|_| image_error(DockerError::Deadline))?
            .map_err(|source| LambdaServiceError::ImageRestoreWorker {
                source: source.into(),
            })?;
        check_restore_deadline(deadline)?;
        result
    }

    fn input(
        snapshot: &LambdaSnapshot,
        region: &str,
        account: &str,
    ) -> Result<(), LambdaServiceError> {
        if snapshot.functions.functions.len() > MAX_IMAGE_FUNCTIONS {
            return Err(invalid());
        }
        let mut work = GuardWork::new();
        work.charge(snapshot.functions.functions.len())?;
        work.charge(snapshot.layers.layers.len())?;
        work.charge(snapshot.event_source_mappings.mappings.len())?;
        let mut versions = 0usize;
        for function in &snapshot.functions.functions {
            FunctionName::parse(&function.name).map_err(|_| invalid())?;
            if function.arn != function_arn(region, account, &function.name) {
                return Err(invalid());
            }
            versions = versions
                .checked_add(1)
                .and_then(|n| n.checked_add(function.versions.len()))
                .ok_or_else(invalid)?;
            if versions > MAX_IMAGE_REVISIONS || function.policy.statements.len() > 128 {
                return Err(invalid());
            }
            work.charge(function.versions.len().checked_add(1).ok_or_else(invalid)?)?;
            for version in once(&function.latest).chain(function.versions.values()) {
                if version.package_type == "Zip" {
                    return Err(LambdaServiceError::InvalidParameter {
                        message: "Docker ZIP archive execution is unavailable".into(),
                    });
                }
            }
            policy(&function.policy, &mut work)?;
        }
        for layer in &snapshot.layers.layers {
            layer_record_guarded(layer, &mut work)?;
        }
        let mut bytes = BoundedBytes::default();
        serde_json::to_writer(&mut bytes, snapshot).map_err(|_| invalid())?;
        let limits = JsonLimits::new(IMAGE_METADATA_BYTES, 64, 65_536, 128 * 1024, 1024)
            .map_err(|_| invalid())?;
        validate(&bytes.0, limits).map_err(|_| invalid())
    }
    pub(crate) async fn validate_image(
        snapshot: LambdaSnapshot,
        deadline: Instant,
        region: &str,
        account: &str,
    ) -> Result<LambdaSnapshot, LambdaServiceError> {
        validate_with(snapshot, deadline, (region.into(), account.into()), input).await
    }
    async fn validate_with(
        snapshot: LambdaSnapshot,
        deadline: Instant,
        scope: (String, String),
        worker: fn(&LambdaSnapshot, &str, &str) -> Result<(), LambdaServiceError>,
    ) -> Result<LambdaSnapshot, LambdaServiceError> {
        check_restore_deadline(deadline)?;
        let permit = timeout_at(deadline, VALIDATORS.acquire())
            .await
            .map_err(|_| image_error(DockerError::Deadline))?
            .map_err(|_| image_error(DockerError::Stopped))?;
        // The read-only job owns input before its slot in drop order, retaining
        // admission through iterative policy destruction even on worker unwind.
        let mut job = spawn_blocking(move || {
            let retained = (snapshot, permit);
            let result = (|| {
                check_restore_deadline(deadline)?;
                worker(&retained.0, &scope.0, &scope.1)?;
                check_restore_deadline(deadline)
            })();
            match result {
                Ok(()) => Ok(retained.0),
                Err(error) => {
                    drop(retained);
                    Err(error)
                }
            }
        });
        // Dropped observations detach only bounded read-only owned CPU work.
        // No late result has a store or publication capability.
        let result = timeout_at(deadline, &mut job)
            .await
            .map_err(|_| image_error(DockerError::Deadline))?
            .map_err(|source| LambdaServiceError::ImageRestoreWorker {
                source: source.into(),
            })?;
        check_restore_deadline(deadline)?;
        result
    }

    #[cfg(test)]
    mod tests {
        use std::{error::Error as _, time::Duration};

        use rustack_lambda_model::error::{LambdaError, LambdaErrorCode};

        use super::*;
        use crate::{
            config::LambdaConfig,
            deployment::image_record_fixture as record,
            executor::ExecutorBackend,
            provider::RustackLambda,
            storage::{FunctionStore, PolicyStatement},
        };

        fn deep_snapshot() -> LambdaSnapshot {
            let store = FunctionStore::new("unused-pure-snapshot-fixture");
            store.insert(record()).unwrap();
            let mut snapshot = LambdaSnapshot {
                functions: store.export_snapshot(),
                ..LambdaSnapshot::default()
            };
            let mut condition = Value::Null;
            for _ in 0..50_000 {
                condition = Value::Array(vec![condition]);
            }
            snapshot
                .functions
                .functions
                .first_mut()
                .unwrap()
                .policy
                .statements
                .push(PolicyStatement {
                    sid: "deep".into(),
                    effect: "Allow".into(),
                    principal: "*".into(),
                    action: "lambda:InvokeFunction".into(),
                    resource: "*".into(),
                    condition: Some(condition),
                });
            snapshot
        }
        #[tokio::test]
        async fn test_should_destroy_deep_rejected_expired_and_unpolled_policy_inputs_without_recursion()
         {
            let error = validate_image(
                deep_snapshot(),
                Instant::now() + Duration::from_secs(30),
                "us-east-1",
                "000000000000",
            )
            .await
            .unwrap_err();
            assert!(matches!(error, LambdaServiceError::InvalidParameter { .. }));
            let error =
                validate_image(deep_snapshot(), Instant::now(), "us-east-1", "000000000000")
                    .await
                    .unwrap_err();
            assert!(
                matches!(error, LambdaServiceError::ImageRestore { source } if matches!(*source, DockerError::Deadline))
            );
            let provider = RustackLambda::new(LambdaConfig {
                executor: ExecutorBackend::Docker,
                ..LambdaConfig::default()
            });
            drop(
                provider.import_snapshot_before(
                    deep_snapshot(),
                    Instant::now() + Duration::from_secs(30),
                ),
            );
        }
        fn panic_worker(_: &LambdaSnapshot, _: &str, _: &str) -> Result<(), LambdaServiceError> {
            panic!("private-image-validator-panic");
        }
        #[tokio::test]
        async fn test_should_preserve_typed_worker_panic_without_exposing_payload_or_parameter_error()
         {
            let error = validate_with(
                LambdaSnapshot::default(),
                Instant::now() + Duration::from_secs(30),
                ("us-east-1".into(), "000000000000".into()),
                panic_worker,
            )
            .await
            .unwrap_err();
            assert!(
                matches!(&error, LambdaServiceError::ImageRestoreWorker { source } if source.is_panic() && !source.is_cancelled())
            );
            assert!(!format!("{error:?}").contains("private-image-validator-panic"));
            let source = error.source().unwrap();
            assert!(!source.to_string().contains("private-image-validator-panic"));
            assert!(source.source().is_none());
            let public: LambdaError = error.into();
            assert_eq!(public.code, LambdaErrorCode::ServiceException);
        }
        #[test]
        fn test_should_share_guard_work_across_ancillary_versions_and_conditions() {
            let mut layer: LayerRecord = serde_json::from_value(serde_json::json!({"name":"work","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:work","nextVersion":2,
                "versions":{"1":{"version":1,"description":"","compatibleRuntimes":[],"compatibleArchitectures":[],"licenseInfo":null,"codeSha256":"unused","codeSize":0,"createdDate":"2026-10-27T00:00:00Z","layerArn":"arn:aws:lambda:us-east-1:000000000000:layer:work","layerVersionArn":"arn:aws:lambda:us-east-1:000000000000:layer:work:1","policy":{"statements":[]}}}})).unwrap();
            let version = layer.versions.get_mut(&1).unwrap();
            for index in 0..128 {
                version.policy.statements.push(PolicyStatement {
                    sid: index.to_string(),
                    effect: "Allow".into(),
                    principal: "*".into(),
                    action: "lambda:GetLayerVersion".into(),
                    resource: "*".into(),
                    condition: Some(Value::Array(vec![Value::Null; 512])),
                });
            }
            let mut work = GuardWork::new();
            assert!(layer_record_guarded(&layer, &mut work).is_err());
            assert!(
                work.visited_nodes > 1
                    && work.visited_nodes <= 65_536
                    && work.visited_nodes < 128 * 513
            );
            layer.versions.get_mut(&1).unwrap().policy = PolicyDocument::default();
            let template = layer.versions.get(&1).unwrap().clone();
            for index in 2..=6 {
                layer.versions.insert(index, template.clone());
            }
            let mut work = GuardWork::new();
            work.remaining = 5;
            assert!(layer_record_guarded(&layer, &mut work).is_err());
            assert_eq!(work.visited_nodes, 0);
            assert_eq!(work.remaining, 5);
        }
        #[tokio::test]
        async fn test_should_classify_canceled_validation_workers_without_raw_join_diagnostics() {
            let task = tokio::spawn(std::future::pending::<()>());
            task.abort();
            let source = crate::error::ImageRestoreWorker::from(task.await.unwrap_err());
            assert!(source.is_cancelled() && !source.is_panic());
            assert!(source.source().is_none());
            let error = LambdaServiceError::ImageRestoreWorker { source };
            let public: LambdaError = error.into();
            assert_eq!(public.code, LambdaErrorCode::ServiceException);
        }
        #[test]
        fn test_should_bound_profile_counts_strings_depth_and_writer_before_effects() {
            let mut bytes = BoundedBytes::default();
            assert!(bytes.write(&vec![0; IMAGE_METADATA_BYTES + 1]).is_err());
            assert_eq!(bytes.0, [] as [u8; 0]);
            let mut snapshot = LambdaSnapshot::default();
            let store = FunctionStore::new("unused-count-fixture");
            store.insert(record()).unwrap();
            let one = store.export_snapshot().functions.pop().unwrap();
            snapshot.functions.functions = vec![one; 257];
            assert!(input(&snapshot, "us-east-1", "000000000000").is_err());
            assert!(
                condition(
                    &Value::String("x".repeat(128 * 1024 + 1)),
                    &mut GuardWork::new()
                )
                .is_err()
            );
            assert!(
                condition(
                    &Value::Array(vec![Value::Null; 65_536]),
                    &mut GuardWork::new()
                )
                .is_err()
            );
        }
    }
}
#[cfg(unix)]
pub(super) use validation::{export_image, validate_image};
