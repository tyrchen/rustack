//! One allowlist defines typed requests, replies and operation correlation.

use std::{fmt, io::Write};

use rustack_core::json::{JsonLimits, from_slice};
use rustack_stepfunctions_model::{StepFunctionsOperation, input, output};

use crate::HttpError;

macro_rules! operations {
    ($($op:ident => $input:ident, $output:ident;)*) => {
        /// Typed boundary requests; DTO contents stay out of diagnostics.
        #[non_exhaustive]
        pub enum Request {$(#[doc=concat!("Typed `",stringify!($op),"` input.")] $op(Box<input::$input>),)*}
        /// Typed replies; only the contained DTO is encoded on the wire.
        #[non_exhaustive]
        pub enum Response {$(#[doc=concat!("Typed `",stringify!($op),"` output.")] $op(Box<output::$output>),)*}
        impl Request {
            /// Preflight/decode and bound a supported request before handler entry.
            ///
            /// # Errors
            /// Rejects unsupported operations, invalid/ambiguous/oversized JSON and field bounds.
            ///
            /// ```
            /// let request=rustack_stepfunctions_http::Request::decode(rustack_stepfunctions_model::StepFunctionsOperation::ListStateMachines,b"{}")?;
            /// assert_eq!(request.operation(),rustack_stepfunctions_model::StepFunctionsOperation::ListStateMachines);
            /// # Ok::<(),rustack_stepfunctions_http::HttpError>(())
            /// ```
            pub fn decode(operation:StepFunctionsOperation, bytes:&[u8])->Result<Self,HttpError> {
                let limits=JsonLimits::new(2*1024*1024,16,4096,1024*1024,128).map_err(HttpError::Json)?;
                let bytes=if bytes.is_empty(){b"{}".as_slice()}else{bytes};
                let request=match operation {$(StepFunctionsOperation::$op=>Self::$op(Box::new(from_slice::<input::$input>(bytes,limits).map_err(HttpError::Json)?)),)* _=>return Err(HttpError::UnsupportedOperation)};
                crate::validation::validate(&request)?; Ok(request)
            }
            /// Recheck bounds for a trusted typed request before domain preparation.
            ///
            /// # Errors
            /// Rejects invalid byte/count limits, business JSON or wire fields.
            ///
            /// ```
            /// let request=rustack_stepfunctions_http::Request::decode(rustack_stepfunctions_model::StepFunctionsOperation::ListStateMachines,b"{}")?;
            /// request.validate()?;
            /// # Ok::<(),rustack_stepfunctions_http::HttpError>(())
            /// ```
            pub fn validate(&self)->Result<(),HttpError>{crate::validation::validate(self)}
            /// Exact modeled operation for this variant.
            #[must_use] pub fn operation(&self)->StepFunctionsOperation {match self {$(Self::$op(_)=>StepFunctionsOperation::$op,)*}}
        }
        impl Response {
            /// Exact modeled operation for correlation before serialization.
            #[must_use] pub fn operation(&self)->StepFunctionsOperation {match self {$(Self::$op(_)=>StepFunctionsOperation::$op,)*}}
            pub(crate) fn encode<W:Write>(&self,writer:W)->Result<(),serde_json::Error> {match self {$(Self::$op(value)=>serde_json::to_writer(writer,value),)*}}
        }
        $(impl From<output::$output> for Response {fn from(value:output::$output)->Self {Self::$op(Box::new(value))}})*
    }
}
operations! {
    CreateStateMachine=>CreateStateMachineInput,CreateStateMachineOutput;
    UpdateStateMachine=>UpdateStateMachineInput,UpdateStateMachineOutput;
    DeleteStateMachine=>DeleteStateMachineInput,DeleteStateMachineOutput;
    DescribeStateMachine=>DescribeStateMachineInput,DescribeStateMachineOutput;
    ListStateMachines=>ListStateMachinesInput,ListStateMachinesOutput;
    ListStateMachineVersions=>ListStateMachineVersionsInput,ListStateMachineVersionsOutput;
    ValidateStateMachineDefinition=>ValidateStateMachineDefinitionInput,ValidateStateMachineDefinitionOutput;
    StartExecution=>StartExecutionInput,StartExecutionOutput;
    DescribeExecution=>DescribeExecutionInput,DescribeExecutionOutput;
    ListExecutions=>ListExecutionsInput,ListExecutionsOutput;
    StopExecution=>StopExecutionInput,StopExecutionOutput;
    GetExecutionHistory=>GetExecutionHistoryInput,GetExecutionHistoryOutput;
    DescribeStateMachineForExecution=>DescribeStateMachineForExecutionInput,DescribeStateMachineForExecutionOutput;
    TagResource=>TagResourceInput,TagResourceOutput;
    UntagResource=>UntagResourceInput,UntagResourceOutput;
    ListTagsForResource=>ListTagsForResourceInput,ListTagsForResourceOutput;
    SendTaskSuccess=>SendTaskSuccessInput,SendTaskSuccessOutput;
    SendTaskFailure=>SendTaskFailureInput,SendTaskFailureOutput;
    SendTaskHeartbeat=>SendTaskHeartbeatInput,SendTaskHeartbeatOutput;
}
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("operation", &self.operation())
            .finish_non_exhaustive()
    }
}
impl fmt::Debug for Response {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("operation", &self.operation())
            .finish_non_exhaustive()
    }
}
