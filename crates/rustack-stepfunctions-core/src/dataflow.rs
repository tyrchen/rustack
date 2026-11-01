//! Pure input/result phases; execution owners retain raw input across resources.

use std::fmt;

use serde_json::Value;

use crate::{
    AslError, CallbackHistory, CallbackParameters, CallbackToken, EvaluationContext, Path, Payload,
    ResultPath, SystemUuidSource, Template, UuidSource,
    expression::Budget,
    payload::{parse_component, validate_component},
};

#[derive(Clone)]
enum Filter {
    Empty,
    Path(Path),
}
#[derive(Clone)]
enum Merge {
    Replace,
    Discard,
    Path(ResultPath),
}

/// Compiled state transformations with explicit original-input/result boundaries.
#[derive(Clone)]
#[non_exhaustive]
pub struct DataFlow {
    input: Filter,
    parameters: Option<Template>,
    selector: Option<Template>,
    result: Merge,
    output: Filter,
}

impl fmt::Debug for DataFlow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("DataFlow(<redacted>)")
    }
}

impl DataFlow {
    /// Preflight a transformation-only JSON object before compiling its fields.
    ///
    /// # Errors
    /// Rejects duplicate/unknown fields, invalid templates and paths.
    ///
    /// ```
    /// let flow=rustack_stepfunctions_core::DataFlow::parse(br#"{"InputPath":"$.data"}"#)?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn parse(input: &[u8]) -> Result<Self, AslError> {
        Self::compile(&parse_component(input)?)
    }

    /// Compile the five transformation fields; the state compiler owns its profile.
    ///
    /// # Errors
    /// Rejects unknown fields, invalid types/paths and normalized-key collisions.
    ///
    /// ```
    /// let flow=rustack_stepfunctions_core::DataFlow::compile(&serde_json::json!({"ResultPath":null}))?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn compile(value: &Value) -> Result<Self, AslError> {
        Self::compile_located(value).map_err(|(_, error)| error)
    }

    pub(crate) fn compile_located(value: &Value) -> Result<Self, (&'static str, AslError)> {
        validate_component(value).map_err(|e| ("", e))?;
        let object = value.as_object().ok_or(("", AslError::InvalidDataFlow))?;
        if object.keys().any(|key| {
            !matches!(
                key.as_str(),
                "InputPath" | "Parameters" | "ResultSelector" | "ResultPath" | "OutputPath"
            )
        }) {
            return Err(("", AslError::InvalidDataFlow));
        }
        let result = match object.get("ResultPath") {
            None => Merge::Replace,
            Some(Value::Null) => Merge::Discard,
            Some(Value::String(path)) => Merge::Path(path.parse().map_err(|e| ("ResultPath", e))?),
            _ => return Err(("ResultPath", AslError::InvalidDataFlow)),
        };
        Ok(Self {
            input: filter(object.get("InputPath")).map_err(|e| ("InputPath", e))?,
            parameters: object
                .get("Parameters")
                .map(Template::compile)
                .transpose()
                .map_err(|e| ("Parameters", e))?,
            selector: object
                .get("ResultSelector")
                .map(Template::compile)
                .transpose()
                .map_err(|e| ("ResultSelector", e))?,
            result,
            output: filter(object.get("OutputPath")).map_err(|e| ("OutputPath", e))?,
        })
    }

    pub(crate) fn node_count(&self) -> usize {
        1_usize
            .saturating_add(self.parameters.as_ref().map_or(0, Template::node_count))
            .saturating_add(self.selector.as_ref().map_or(0, Template::node_count))
    }

    /// Apply `InputPath` then Parameters before calling a resource.
    ///
    /// # Errors
    /// Returns typed selection/template/budget failures and source failures.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{DataFlow,Payload,EvaluationContext};
    /// let original=Payload::parse(br#"{"data":{"n":1},"keep":true}"#)?;
    /// let flow=DataFlow::parse(br#"{"InputPath":"$.data"}"#)?;
    /// assert_eq!(flow.prepare(&original,&EvaluationContext::from(&original))?.value(),&serde_json::json!({"n":1}));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn prepare(
        &self,
        original: &Payload,
        context: &EvaluationContext,
    ) -> Result<Payload, AslError> {
        self.prepare_with_source(original, context, &mut SystemUuidSource)
    }

    /// Prepare input with an explicit UUID source and one shared phase budget.
    ///
    /// # Errors
    /// Returns data/source failures; original input is unchanged on every outcome.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{DataFlow,Payload,EvaluationContext,SystemUuidSource};
    /// let input=Payload::parse(b"null")?;
    /// let flow=DataFlow::parse(b"{}")?;
    /// flow.prepare_with_source(&input,&EvaluationContext::from(&input),&mut SystemUuidSource::default())?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn prepare_with_source<S: UuidSource + ?Sized>(
        &self,
        original: &Payload,
        context: &EvaluationContext,
        source: &mut S,
    ) -> Result<Payload, AslError> {
        let mut budget = Budget::new();
        let input = apply(&self.input, original, context, &mut budget)?;
        if let Some(template) = &self.parameters {
            template.evaluate_scoped(&input, context, source, &mut budget)
        } else {
            Ok(input)
        }
    }

    /// Prepare callback invocation/history projections with one scoped credential.
    ///
    /// Call only after real callback registration and under bounded CPU admission.
    /// The transient context never changes the supplied original/base context.
    ///
    /// # Errors
    /// Rejects stale reserved Task context, invalid transformations, source failure
    /// and independent context/outbound/history byte/depth/member/work limits.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{CallbackTask, DataFlow, Payload, EvaluationContext, SystemUuidSource};
    /// # fn prepare(task: &CallbackTask) -> Result<(),Box<dyn std::error::Error>> {
    /// let flow = DataFlow::parse(br#"{"Parameters":{"token.$":"$$.Task.Token","n.$":"$.n"}}"#)?;
    /// let input = Payload::parse(br#"{"n":21}"#)?;
    /// let result = flow.prepare_callback_with_source(&input, &EvaluationContext::from_value(&serde_json::json!({}))?, task.token(), &mut SystemUuidSource::default())?;
    /// assert!(result.outbound().value()["token"].is_string());
    /// assert!(result.history().payload().value()["token"].is_null());
    /// assert!(result.history().redacted());
    /// # Ok(()) }
    /// ```
    pub fn prepare_callback_with_source<S: UuidSource + ?Sized>(
        &self,
        original: &Payload,
        base_context: &EvaluationContext,
        token: &CallbackToken,
        source: &mut S,
    ) -> Result<CallbackParameters, AslError> {
        let context = base_context.for_callback(token)?;
        let mut budget = Budget::new();
        let input = apply(&self.input, original, &context, &mut budget)?;
        let sensitive = match &self.input {
            Filter::Empty => false,
            Filter::Path(path) => path.callback_sensitive(false),
        };
        if let Some(template) = &self.parameters {
            template.evaluate_callback(&input, &context, source, &mut budget, sensitive)
        } else {
            let history = if sensitive {
                Payload::parse(b"null")?
            } else {
                input.clone()
            };
            budget.result(&history)?;
            Ok(CallbackParameters {
                outbound: input,
                history: CallbackHistory {
                    payload: history,
                    redacted: sensitive,
                },
            })
        }
    }

    /// Select raw result, merge into original state input, then apply `OutputPath`.
    ///
    /// # Errors
    /// Returns result-selector, merge/output, source or intermediate-limit failures.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{DataFlow,Payload,EvaluationContext};
    /// let original=Payload::parse(br#"{"keep":1}"#)?;
    /// let result=Payload::parse(b"2")?;
    /// let flow=DataFlow::parse(br#"{"ResultPath":"$.answer"}"#)?;
    /// assert_eq!(flow.finish(&original,&result,&EvaluationContext::from(&original))?.value(),&serde_json::json!({"keep":1,"answer":2}));
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn finish(
        &self,
        original: &Payload,
        raw_result: &Payload,
        context: &EvaluationContext,
    ) -> Result<Payload, AslError> {
        self.finish_with_source(original, raw_result, context, &mut SystemUuidSource)
    }

    /// Finish with an explicit UUID source and a shared result-phase budget.
    ///
    /// # Errors
    /// Returns typed failures without changing original/result/context values.
    ///
    /// ```
    /// use rustack_stepfunctions_core::{DataFlow,Payload,EvaluationContext,SystemUuidSource};
    /// let input=Payload::parse(b"null")?;
    /// DataFlow::parse(b"{}")?.finish_with_source(&input,&input,&EvaluationContext::from(&input),&mut SystemUuidSource::default())?;
    /// # Ok::<(), rustack_stepfunctions_core::AslError>(())
    /// ```
    pub fn finish_with_source<S: UuidSource + ?Sized>(
        &self,
        original: &Payload,
        raw_result: &Payload,
        context: &EvaluationContext,
        source: &mut S,
    ) -> Result<Payload, AslError> {
        let mut budget = Budget::new();
        let selected = if let Some(selector) = &self.selector {
            selector.evaluate_scoped(raw_result, context, source, &mut budget)?
        } else {
            raw_result.clone()
        };
        budget.work(1)?;
        let merged = match &self.result {
            Merge::Replace => selected,
            Merge::Discard => original.clone(),
            Merge::Path(path) => path.merge(original, &selected)?,
        };
        budget.result(&merged)?;
        apply(&self.output, &merged, context, &mut budget)
    }
}

fn filter(value: Option<&Value>) -> Result<Filter, AslError> {
    match value {
        None => Ok(Filter::Path("$".parse()?)),
        Some(Value::Null) => Ok(Filter::Empty),
        Some(Value::String(path)) => Ok(Filter::Path(path.parse()?)),
        _ => Err(AslError::InvalidDataFlow),
    }
}

fn apply(
    filter: &Filter,
    input: &Payload,
    context: &EvaluationContext,
    budget: &mut Budget,
) -> Result<Payload, AslError> {
    match filter {
        Filter::Path(path) => budget.path(path, input, context),
        Filter::Empty => {
            budget.work(1)?;
            let output = Payload::parse(b"{}")?;
            budget.result(&output)?;
            Ok(output)
        }
    }
}

#[cfg(test)]
#[path = "dataflow/callback_tests.rs"]
mod callback_tests;
#[cfg(test)]
mod tests;
