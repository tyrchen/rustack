//! SNS handler implementation bridging HTTP to business logic.
//!
//! Parses form-urlencoded request bodies, dispatches to the provider,
//! and serializes XML responses following the awsQuery protocol.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use bytes::Bytes;

use ruststack_sns_http::body::SnsResponseBody;
use ruststack_sns_http::dispatch::SnsHandler;
use ruststack_sns_http::request::{
    get_optional_bool, get_optional_param, get_required_param, parse_attributes_map,
    parse_form_params, parse_message_attributes, parse_publish_batch_entries, parse_string_list,
    parse_tag_list,
};
use ruststack_sns_http::response::{XmlWriter, xml_response};
use ruststack_sns_model::error::SnsError;
use ruststack_sns_model::input::{
    ConfirmSubscriptionInput, CreateTopicInput, DeleteTopicInput, GetSubscriptionAttributesInput,
    GetTopicAttributesInput, ListSubscriptionsByTopicInput, ListSubscriptionsInput,
    ListTagsForResourceInput, ListTopicsInput, PublishBatchInput, PublishInput,
    SetSubscriptionAttributesInput, SetTopicAttributesInput, SubscribeInput, TagResourceInput,
    UnsubscribeInput, UntagResourceInput,
};
use ruststack_sns_model::operations::SnsOperation;
use ruststack_sns_model::types::Subscription;

use crate::provider::RustStackSns;

/// Handler that bridges the HTTP layer to the SNS provider.
#[derive(Debug)]
pub struct RustStackSnsHandler {
    provider: Arc<RustStackSns>,
}

impl RustStackSnsHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackSns>) -> Self {
        Self { provider }
    }
}

impl SnsHandler for RustStackSnsHandler {
    fn handle_operation(
        &self,
        op: SnsOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<SnsResponseBody>, SnsError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(provider.as_ref(), op, &body).await })
    }
}

/// Dispatch an SNS operation to the appropriate handler method.
#[allow(clippy::too_many_lines)] // Match dispatch with one arm per SNS operation.
async fn dispatch(
    provider: &RustStackSns,
    op: SnsOperation,
    body: &[u8],
) -> Result<http::Response<SnsResponseBody>, SnsError> {
    let params = parse_form_params(body);
    let request_id = uuid::Uuid::new_v4().to_string();

    match op {
        SnsOperation::CreateTopic => {
            let input = deserialize_create_topic(&params)?;
            let output = provider.create_topic(input)?;
            let mut w = XmlWriter::new();
            w.start_response("CreateTopic");
            w.start_result("CreateTopic");
            w.write_element("TopicArn", &output.topic_arn);
            w.end_element("CreateTopicResult");
            w.write_response_metadata(&request_id);
            w.end_element("CreateTopicResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::DeleteTopic => {
            let input = deserialize_delete_topic(&params)?;
            let _output = provider.delete_topic(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("DeleteTopic");
            w.write_response_metadata(&request_id);
            w.end_element("DeleteTopicResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::GetTopicAttributes => {
            let input = deserialize_get_topic_attributes(&params)?;
            let output = provider.get_topic_attributes(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetTopicAttributes");
            w.start_result("GetTopicAttributes");
            write_attributes_xml(&mut w, &output.attributes);
            w.end_element("GetTopicAttributesResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetTopicAttributesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::SetTopicAttributes => {
            let input = deserialize_set_topic_attributes(&params)?;
            let _output = provider.set_topic_attributes(input)?;
            let mut w = XmlWriter::new();
            w.start_response("SetTopicAttributes");
            w.write_response_metadata(&request_id);
            w.end_element("SetTopicAttributesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::ListTopics => {
            let input = deserialize_list_topics(&params);
            let output = provider.list_topics(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListTopics");
            w.start_result("ListTopics");
            w.raw("<Topics>");
            for topic in &output.topics {
                w.raw("<member>");
                w.write_element("TopicArn", &topic.topic_arn);
                w.raw("</member>");
            }
            w.raw("</Topics>");
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListTopicsResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListTopicsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::Subscribe => {
            let input = deserialize_subscribe(&params)?;
            let output = provider.subscribe(input)?;
            let mut w = XmlWriter::new();
            w.start_response("Subscribe");
            w.start_result("Subscribe");
            w.write_optional_element("SubscriptionArn", output.subscription_arn.as_deref());
            w.end_element("SubscribeResult");
            w.write_response_metadata(&request_id);
            w.end_element("SubscribeResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::Unsubscribe => {
            let input = deserialize_unsubscribe(&params)?;
            let _output = provider.unsubscribe(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("Unsubscribe");
            w.write_response_metadata(&request_id);
            w.end_element("UnsubscribeResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::ConfirmSubscription => {
            let input = deserialize_confirm_subscription(&params)?;
            let output = provider.confirm_subscription(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("ConfirmSubscription");
            w.start_result("ConfirmSubscription");
            w.write_optional_element("SubscriptionArn", output.subscription_arn.as_deref());
            w.end_element("ConfirmSubscriptionResult");
            w.write_response_metadata(&request_id);
            w.end_element("ConfirmSubscriptionResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::GetSubscriptionAttributes => {
            let input = deserialize_get_subscription_attributes(&params)?;
            let output = provider.get_subscription_attributes(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("GetSubscriptionAttributes");
            w.start_result("GetSubscriptionAttributes");
            write_attributes_xml(&mut w, &output.attributes);
            w.end_element("GetSubscriptionAttributesResult");
            w.write_response_metadata(&request_id);
            w.end_element("GetSubscriptionAttributesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::SetSubscriptionAttributes => {
            let input = deserialize_set_subscription_attributes(&params)?;
            let _output = provider.set_subscription_attributes(input)?;
            let mut w = XmlWriter::new();
            w.start_response("SetSubscriptionAttributes");
            w.write_response_metadata(&request_id);
            w.end_element("SetSubscriptionAttributesResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::ListSubscriptions => {
            let input = deserialize_list_subscriptions(&params);
            let output = provider.list_subscriptions(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListSubscriptions");
            w.start_result("ListSubscriptions");
            write_subscriptions_xml(&mut w, &output.subscriptions);
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListSubscriptionsResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListSubscriptionsResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::ListSubscriptionsByTopic => {
            let input = deserialize_list_subscriptions_by_topic(&params)?;
            let output = provider.list_subscriptions_by_topic(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListSubscriptionsByTopic");
            w.start_result("ListSubscriptionsByTopic");
            write_subscriptions_xml(&mut w, &output.subscriptions);
            w.write_optional_element("NextToken", output.next_token.as_deref());
            w.end_element("ListSubscriptionsByTopicResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListSubscriptionsByTopicResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::Publish => {
            let input = deserialize_publish(&params)?;
            let output = provider.publish(input).await?;
            let mut w = XmlWriter::new();
            w.start_response("Publish");
            w.start_result("Publish");
            w.write_optional_element("MessageId", output.message_id.as_deref());
            w.write_optional_element("SequenceNumber", output.sequence_number.as_deref());
            w.end_element("PublishResult");
            w.write_response_metadata(&request_id);
            w.end_element("PublishResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::PublishBatch => {
            let input = deserialize_publish_batch(&params)?;
            let output = provider.publish_batch(input).await?;
            let mut w = XmlWriter::new();
            w.start_response("PublishBatch");
            w.start_result("PublishBatch");
            w.raw("<Successful>");
            for entry in &output.successful {
                w.raw("<member>");
                w.write_element("Id", &entry.id);
                w.write_element("MessageId", &entry.message_id);
                w.write_optional_element("SequenceNumber", entry.sequence_number.as_deref());
                w.raw("</member>");
            }
            w.raw("</Successful>");
            w.raw("<Failed>");
            for entry in &output.failed {
                w.raw("<member>");
                w.write_element("Id", &entry.id);
                w.write_element("Code", &entry.code);
                w.write_element("Message", &entry.message);
                w.write_bool_element("SenderFault", entry.sender_fault);
                w.raw("</member>");
            }
            w.raw("</Failed>");
            w.end_element("PublishBatchResult");
            w.write_response_metadata(&request_id);
            w.end_element("PublishBatchResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::TagResource => {
            let input = deserialize_tag_resource(&params)?;
            let _output = provider.tag_resource(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("TagResource");
            w.write_response_metadata(&request_id);
            w.end_element("TagResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::UntagResource => {
            let input = deserialize_untag_resource(&params)?;
            let _output = provider.untag_resource(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("UntagResource");
            w.write_response_metadata(&request_id);
            w.end_element("UntagResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        SnsOperation::ListTagsForResource => {
            let input = deserialize_list_tags_for_resource(&params)?;
            let output = provider.list_tags_for_resource(&input)?;
            let mut w = XmlWriter::new();
            w.start_response("ListTagsForResource");
            w.start_result("ListTagsForResource");
            w.raw("<Tags>");
            for tag in &output.tags {
                w.raw("<member>");
                w.write_element("Key", &tag.key);
                w.write_element("Value", &tag.value);
                w.raw("</member>");
            }
            w.raw("</Tags>");
            w.end_element("ListTagsForResourceResult");
            w.write_response_metadata(&request_id);
            w.end_element("ListTagsForResourceResponse");
            Ok(xml_response(w.into_string(), &request_id))
        }

        // All other operations are not yet implemented.
        _ => Err(SnsError::invalid_parameter(format!(
            "Operation not yet supported: {op}"
        ))),
    }
}

// ---------------------------------------------------------------------------
// XML helpers
// ---------------------------------------------------------------------------

/// Write a `<Attributes>` block with `<entry><key>...</key><value>...</value></entry>`.
fn write_attributes_xml(w: &mut XmlWriter, attrs: &HashMap<String, String>) {
    w.raw("<Attributes>");
    // Sort keys for deterministic output.
    let mut keys: Vec<&String> = attrs.keys().collect();
    keys.sort();
    for k in keys {
        if let Some(v) = attrs.get(k) {
            w.raw("<entry>");
            w.write_element("key", k);
            w.write_element("value", v);
            w.raw("</entry>");
        }
    }
    w.raw("</Attributes>");
}

/// Write a `<Subscriptions>` block with `<member>` elements.
fn write_subscriptions_xml(w: &mut XmlWriter, subs: &[Subscription]) {
    w.raw("<Subscriptions>");
    for sub in subs {
        w.raw("<member>");
        w.write_element("SubscriptionArn", &sub.subscription_arn);
        w.write_element("Owner", &sub.owner);
        w.write_element("Protocol", &sub.protocol);
        w.write_element("Endpoint", &sub.endpoint);
        w.write_element("TopicArn", &sub.topic_arn);
        w.raw("</member>");
    }
    w.raw("</Subscriptions>");
}

// ---------------------------------------------------------------------------
// Deserializers: form params -> input structs
// ---------------------------------------------------------------------------

fn deserialize_create_topic(params: &[(String, String)]) -> Result<CreateTopicInput, SnsError> {
    let name = get_required_param(params, "Name")?.to_owned();
    let attributes = parse_attributes_map(params, "Attributes")?;
    let tags = parse_tag_list(params, "Tags")?;
    let data_protection_policy =
        get_optional_param(params, "DataProtectionPolicy").map(str::to_owned);

    Ok(CreateTopicInput {
        name,
        attributes,
        tags,
        data_protection_policy,
    })
}

fn deserialize_delete_topic(params: &[(String, String)]) -> Result<DeleteTopicInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    Ok(DeleteTopicInput { topic_arn })
}

fn deserialize_get_topic_attributes(
    params: &[(String, String)],
) -> Result<GetTopicAttributesInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    Ok(GetTopicAttributesInput { topic_arn })
}

fn deserialize_set_topic_attributes(
    params: &[(String, String)],
) -> Result<SetTopicAttributesInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    let attribute_name = get_required_param(params, "AttributeName")?.to_owned();
    let attribute_value = get_optional_param(params, "AttributeValue").map(str::to_owned);
    Ok(SetTopicAttributesInput {
        topic_arn,
        attribute_name,
        attribute_value,
    })
}

fn deserialize_list_topics(params: &[(String, String)]) -> ListTopicsInput {
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    ListTopicsInput { next_token }
}

fn deserialize_subscribe(params: &[(String, String)]) -> Result<SubscribeInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    let protocol = get_required_param(params, "Protocol")?.to_owned();
    let endpoint = get_optional_param(params, "Endpoint").map(str::to_owned);
    let return_subscription_arn =
        get_optional_bool(params, "ReturnSubscriptionArn").unwrap_or(false);
    let attributes = parse_attributes_map(params, "Attributes")?;

    Ok(SubscribeInput {
        topic_arn,
        protocol,
        endpoint,
        return_subscription_arn,
        attributes,
    })
}

fn deserialize_unsubscribe(params: &[(String, String)]) -> Result<UnsubscribeInput, SnsError> {
    let subscription_arn = get_required_param(params, "SubscriptionArn")?.to_owned();
    Ok(UnsubscribeInput { subscription_arn })
}

fn deserialize_confirm_subscription(
    params: &[(String, String)],
) -> Result<ConfirmSubscriptionInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    let token = get_required_param(params, "Token")?.to_owned();
    let authenticate_on_unsubscribe =
        get_optional_param(params, "AuthenticateOnUnsubscribe").map(str::to_owned);
    Ok(ConfirmSubscriptionInput {
        topic_arn,
        token,
        authenticate_on_unsubscribe,
    })
}

fn deserialize_get_subscription_attributes(
    params: &[(String, String)],
) -> Result<GetSubscriptionAttributesInput, SnsError> {
    let subscription_arn = get_required_param(params, "SubscriptionArn")?.to_owned();
    Ok(GetSubscriptionAttributesInput { subscription_arn })
}

fn deserialize_set_subscription_attributes(
    params: &[(String, String)],
) -> Result<SetSubscriptionAttributesInput, SnsError> {
    let subscription_arn = get_required_param(params, "SubscriptionArn")?.to_owned();
    let attribute_name = get_required_param(params, "AttributeName")?.to_owned();
    let attribute_value = get_optional_param(params, "AttributeValue").map(str::to_owned);
    Ok(SetSubscriptionAttributesInput {
        subscription_arn,
        attribute_name,
        attribute_value,
    })
}

fn deserialize_list_subscriptions(params: &[(String, String)]) -> ListSubscriptionsInput {
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    ListSubscriptionsInput { next_token }
}

fn deserialize_list_subscriptions_by_topic(
    params: &[(String, String)],
) -> Result<ListSubscriptionsByTopicInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    let next_token = get_optional_param(params, "NextToken").map(str::to_owned);
    Ok(ListSubscriptionsByTopicInput {
        topic_arn,
        next_token,
    })
}

fn deserialize_publish(params: &[(String, String)]) -> Result<PublishInput, SnsError> {
    let topic_arn = get_optional_param(params, "TopicArn").map(str::to_owned);
    let target_arn = get_optional_param(params, "TargetArn").map(str::to_owned);
    let phone_number = get_optional_param(params, "PhoneNumber").map(str::to_owned);
    let message = get_required_param(params, "Message")?.to_owned();
    let subject = get_optional_param(params, "Subject").map(str::to_owned);
    let message_structure = get_optional_param(params, "MessageStructure").map(str::to_owned);
    let message_group_id = get_optional_param(params, "MessageGroupId").map(str::to_owned);
    let message_deduplication_id =
        get_optional_param(params, "MessageDeduplicationId").map(str::to_owned);
    let message_attributes = parse_message_attributes(params, "MessageAttributes")?;

    Ok(PublishInput {
        topic_arn,
        target_arn,
        phone_number,
        message,
        subject,
        message_structure,
        message_attributes,
        message_group_id,
        message_deduplication_id,
    })
}

fn deserialize_publish_batch(params: &[(String, String)]) -> Result<PublishBatchInput, SnsError> {
    let topic_arn = get_required_param(params, "TopicArn")?.to_owned();
    let entries = parse_publish_batch_entries(params)?;
    Ok(PublishBatchInput {
        topic_arn,
        publish_batch_request_entries: entries,
    })
}

fn deserialize_tag_resource(params: &[(String, String)]) -> Result<TagResourceInput, SnsError> {
    let resource_arn = get_required_param(params, "ResourceArn")?.to_owned();
    let tags = parse_tag_list(params, "Tags")?;
    Ok(TagResourceInput { resource_arn, tags })
}

fn deserialize_untag_resource(params: &[(String, String)]) -> Result<UntagResourceInput, SnsError> {
    let resource_arn = get_required_param(params, "ResourceArn")?.to_owned();
    let tag_keys = parse_string_list(params, "TagKeys");
    Ok(UntagResourceInput {
        resource_arn,
        tag_keys,
    })
}

fn deserialize_list_tags_for_resource(
    params: &[(String, String)],
) -> Result<ListTagsForResourceInput, SnsError> {
    let resource_arn = get_required_param(params, "ResourceArn")?.to_owned();
    Ok(ListTagsForResourceInput { resource_arn })
}
