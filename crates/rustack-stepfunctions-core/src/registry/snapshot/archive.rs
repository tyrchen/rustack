//! Move-only service archive parts; joining never grants runtime validity.

use std::{collections::BTreeMap, mem};

use serde::{Deserialize, Serialize};

use super::{HistoryRow, RegistrySnapshot, decode};
use crate::registry::{RegistryError, domain::redacted};

/// Data-only schema1 registry metadata with histories in a separate keyed part.
///
/// Decode under the embedding's bounded redacting adapter. The inner document
/// retains the original source/identity/payload bounds and redacted Debug.
///
/// ```no_run
/// # fn split(document:rustack_stepfunctions_core::RegistrySnapshot)->Result<(),rustack_stepfunctions_core::RegistryError>{
/// let (metadata, histories) = document.into_archive_parts()?;
/// # Ok(()) }
/// ```
#[derive(Serialize, Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct RegistryMetadata(RegistrySnapshot);
redacted!(RegistryMetadata);

/// Versioned bounded keyed histories for a registry's service data archive.
///
/// No compiled state, credential, live handle or monotonic time is serialized.
///
/// ```no_run
/// # fn join(metadata:rustack_stepfunctions_core::RegistryMetadata,histories:rustack_stepfunctions_core::RegistryHistories)->Result<(),rustack_stepfunctions_core::RegistryError>{
/// let document=rustack_stepfunctions_core::RegistrySnapshot::from_archive_parts(metadata,histories)?;
/// # Ok(()) }
/// ```
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[non_exhaustive]
pub struct RegistryHistories {
    schema_version: u32,
    #[serde(deserialize_with = "decode::rows::<_, _, 100000>")]
    histories: Vec<ExecutionHistory>,
}
redacted!(RegistryHistories);

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExecutionHistory {
    #[serde(deserialize_with = "decode::text::<_, 256>")]
    arn: String,
    #[serde(deserialize_with = "decode::rows::<_, _, 25000>")]
    history: Vec<HistoryRow>,
}
redacted!(ExecutionHistory);

impl RegistrySnapshot {
    /// Move durable histories into a separately encoded service data part.
    ///
    /// This preserves the original schema1 metadata shape with empty histories.
    /// The embedding owns bounded CPU/encoding and source-copy admission.
    ///
    /// # Errors
    /// Rejects allocation failure; no partial parts escape.
    ///
    /// ```no_run
    /// # fn split(document:rustack_stepfunctions_core::RegistrySnapshot){let parts=document.into_archive_parts();}
    /// ```
    pub fn into_archive_parts(
        mut self,
    ) -> Result<(RegistryMetadata, RegistryHistories), RegistryError> {
        let mut histories = Vec::new();
        histories
            .try_reserve_exact(self.executions.len())
            .map_err(|_| RegistryError::Capacity)?;
        for execution in &mut self.executions {
            let mut arn = String::new();
            arn.try_reserve_exact(execution.arn.len())
                .map_err(|_| RegistryError::Capacity)?;
            arn.push_str(&execution.arn);
            histories.push(ExecutionHistory {
                arn,
                history: mem::take(&mut execution.history),
            });
        }
        Ok((
            RegistryMetadata(self),
            RegistryHistories {
                schema_version: 1,
                histories,
            },
        ))
    }
    /// Join one exact keyed history part before full registry preparation.
    ///
    /// This establishes cross-part shape only. The resulting document still
    /// requires `Registry::prepare_snapshot` for domain/profile/quota validity.
    ///
    /// # Errors
    /// Rejects unknown history schema, nonempty metadata histories, missing,
    /// duplicate, orphan or ambiguous execution history bindings.
    ///
    /// ```no_run
    /// # fn join(metadata:rustack_stepfunctions_core::RegistryMetadata,histories:rustack_stepfunctions_core::RegistryHistories){let document=rustack_stepfunctions_core::RegistrySnapshot::from_archive_parts(metadata,histories);}
    /// ```
    pub fn from_archive_parts(
        metadata: RegistryMetadata,
        histories: RegistryHistories,
    ) -> Result<Self, RegistryError> {
        let RegistryMetadata(mut document) = metadata;
        if histories.schema_version != 1 || histories.histories.len() != document.executions.len() {
            return Err(RegistryError::Validation);
        }
        let mut rows = BTreeMap::new();
        for row in histories.histories {
            if rows.insert(row.arn, row.history).is_some() {
                return Err(RegistryError::Validation);
            }
        }
        for execution in &mut document.executions {
            if !execution.history.is_empty() {
                return Err(RegistryError::Validation);
            }
            execution.history = rows
                .remove(&execution.arn)
                .ok_or(RegistryError::Validation)?;
        }
        if !rows.is_empty() {
            return Err(RegistryError::Validation);
        }
        Ok(document)
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use rstest::rstest;
    use serde::de::DeserializeOwned;
    use serde_json::{Value, json};

    use super::*;
    use crate::{
        RegistryCommand, RegistryLimits, RegistryResult, SnapshotLimits,
        registry::tests::{deadline, machine, observe, registry, result, start},
    };

    type TestResult = Result<(), Box<dyn Error>>;
    async fn document(empty: bool) -> Result<RegistrySnapshot, Box<dyn Error>> {
        let (source, mut owner) = registry(RegistryLimits::default(), None);
        if !empty {
            machine(&source, 7, false).await;
            observe(start(&source, "stored", " {\"raw\":1}\n").await?).await;
            observe(start(&source, "other", "{}").await?).await;
        }
        owner.shutdown(deadline()).await?;
        Ok(source.export_snapshot(SnapshotLimits::default())?)
    }
    fn cbor<T: Serialize + DeserializeOwned>(value: &T) -> Result<T, Box<dyn Error>> {
        let mut bytes = Vec::new();
        ciborium::into_writer(value, &mut bytes)?;
        Ok(ciborium::from_reader(bytes.as_slice())?)
    }
    #[rstest]
    #[case(false)]
    #[case(true)]
    #[tokio::test]
    async fn test_should_round_trip_real_split_parts_and_restore(
        #[case] empty: bool,
    ) -> TestResult {
        let document = document(empty).await?;
        let expected = serde_json::to_value(&document)?;
        let (metadata, histories) = document.into_archive_parts()?;
        assert!(
            metadata
                .0
                .executions
                .iter()
                .all(|execution| execution.history.is_empty())
        );
        let joined = RegistrySnapshot::from_archive_parts(cbor(&metadata)?, cbor(&histories)?)?;
        assert_eq!(serde_json::to_value(&joined)?, expected);
        let (target, mut owner) = registry(RegistryLimits::default(), None);
        let prepared = target.prepare_snapshot(joined, SnapshotLimits::default())?;
        assert!(matches!(
            target.restore_snapshot(prepared)?.wait(deadline()).await?,
            RegistryResult::Restored
        ));
        let RegistryResult::Machines(machines) = result(&target, RegistryCommand::Machines).await?
        else {
            return Err("missing restored list".into());
        };
        assert_eq!(machines.len(), usize::from(!empty));
        owner.shutdown(deadline()).await?;
        Ok(())
    }
    #[rstest]
    #[case("schema")]
    #[case("missing")]
    #[case("orphan")]
    #[case("duplicate")]
    #[case("inline-history")]
    #[tokio::test]
    async fn test_should_reject_ambiguous_or_incomplete_parts(
        #[case] corruption: &str,
    ) -> TestResult {
        let (metadata, histories) = document(false).await?.into_archive_parts()?;
        let mut metadata = serde_json::to_value(metadata)?;
        let mut histories = serde_json::to_value(histories)?;
        match corruption {
            "schema" => histories["schemaVersion"] = json!(2),
            "missing" => histories["histories"] = json!([]),
            "orphan" => {
                histories["histories"][0]["arn"] =
                    json!("arn:aws:states:us-east-1:000000000000:execution:Machine:orphan");
            }
            "duplicate" => {
                let row = histories
                    .pointer("/histories/0")
                    .cloned()
                    .ok_or("missing history")?;
                histories
                    .get_mut("histories")
                    .and_then(Value::as_array_mut)
                    .ok_or("missing rows")?
                    .get_mut(1)
                    .ok_or("missing second history")?
                    .clone_from(&row);
            }
            "inline-history" => {
                metadata["executions"][0]["history"] = histories["histories"][0]["history"].clone();
            }
            _ => return Err("unknown case".into()),
        }
        assert!(
            RegistrySnapshot::from_archive_parts(
                serde_json::from_value(metadata)?,
                serde_json::from_value(histories)?
            )
            .is_err()
        );
        Ok(())
    }
}
