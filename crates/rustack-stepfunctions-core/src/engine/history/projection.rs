//! Data omission keeps event identity/type/causality unchanged.

use rustack_stepfunctions_model::types::HistoryEvent;

pub(super) fn omit_data(event: &mut HistoryEvent) {
    if let Some(d) = &mut event.execution_started_event_details {
        d.input = None;
        d.input_details = None;
    }
    if let Some(d) = &mut event.execution_succeeded_event_details {
        d.output = None;
        d.output_details = None;
    }
    if let Some(d) = &mut event.state_entered_event_details {
        d.input = None;
        d.input_details = None;
    }
    if let Some(d) = &mut event.state_exited_event_details {
        d.output = None;
        d.output_details = None;
    }
    if let Some(d) = &mut event.lambda_function_scheduled_event_details {
        d.input = None;
        d.input_details = None;
    }
    if let Some(d) = &mut event.lambda_function_succeeded_event_details {
        d.output = None;
        d.output_details = None;
    }
    if let Some(d) = &mut event.task_succeeded_event_details {
        d.output = None;
        d.output_details = None;
    }
    // Parameters is required within the Smithy details type. Omit that optional
    // details object as a whole instead of fabricating a replacement JSON value.
    event.task_scheduled_event_details = None;
}
