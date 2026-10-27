//! Borrow archive projections while map entry guards protect dynamic values.

use std::{borrow::Borrow, fmt};

use base64::{display::Base64Display, engine::general_purpose::STANDARD};
use bytes::Bytes;
use dashmap::DashMap;
use serde::{
    Serialize, Serializer,
    ser::{Error as _, SerializeSeq},
};

use super::{
    EventSourceMappingRecord, EventSourceMappingStore, FunctionRecord, FunctionStore, LayerRecord,
    LayerStore,
};
use crate::{artifact::ImageArtifact, deployment::DeploymentState, error::LambdaServiceError};

type Guard<T> = fn(&T) -> Result<(), LambdaServiceError>;
struct Records<'a, T> {
    values: &'a DashMap<String, T>,
    guard: Guard<T>,
}
impl<T> Serialize for Records<'_, T>
where
    T: Serialize + Send + Sync,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut sequence = serializer.serialize_seq(Some(self.values.len()))?;
        for record in self.values {
            (self.guard)(record.value())
                .map_err(|_| S::Error::custom("invalid image archive record"))?;
            sequence.serialize_element(record.value())?;
        }
        sequence.end()
    }
}
impl<T> fmt::Debug for Records<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Records(<borrowed>)")
    }
}
#[derive(Debug, Serialize)]
pub(crate) struct FunctionSnapshotRef<'a> {
    functions: Records<'a, FunctionRecord>,
}
#[derive(Debug, Serialize)]
pub(crate) struct LayerSnapshotRef<'a> {
    layers: Records<'a, LayerRecord>,
}
#[derive(Debug, Serialize)]
pub(crate) struct MappingSnapshotRef<'a> {
    mappings: Records<'a, EventSourceMappingRecord>,
}
impl FunctionStore {
    pub(crate) fn borrow_snapshot(&self, guard: Guard<FunctionRecord>) -> FunctionSnapshotRef<'_> {
        FunctionSnapshotRef {
            functions: Records {
                values: &self.functions,
                guard,
            },
        }
    }
}
impl LayerStore {
    pub(crate) fn borrow_snapshot(&self, guard: Guard<LayerRecord>) -> LayerSnapshotRef<'_> {
        LayerSnapshotRef {
            layers: Records {
                values: &self.layers,
                guard,
            },
        }
    }
}
impl EventSourceMappingStore {
    pub(crate) fn borrow_snapshot(&self) -> MappingSnapshotRef<'_> {
        MappingSnapshotRef {
            mappings: Records {
                values: &self.mappings,
                guard: |_| Ok(()),
            },
        }
    }
}
pub(super) fn image<S: Serializer>(
    value: &ImageArtifact,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    value.resolved().serialize(serializer)
}
pub(super) fn zip<S: Serializer, B: Borrow<Option<Bytes>>>(
    value: B,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if let Some(bytes) = value.borrow() {
        serializer.collect_str(&Base64Display::new(bytes, &STANDARD))
    } else {
        serializer.serialize_none()
    }
}
pub(super) fn deployment<S: Serializer>(
    value: &DeploymentState,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct State<'a> {
        state: &'a str,
        deployment: &'a DeploymentState,
    }
    State {
        state: value.state(),
        deployment: value,
    }
    .serialize(serializer)
}
