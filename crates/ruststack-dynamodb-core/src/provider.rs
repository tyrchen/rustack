//! DynamoDB provider implementing all MVP operations.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use ruststack_dynamodb_model::{
    AttributeValue,
    error::DynamoDBError,
    input::{
        BatchGetItemInput, BatchWriteItemInput, CreateTableInput, DeleteItemInput,
        DeleteTableInput, DescribeEndpointsInput, DescribeLimitsInput, DescribeTableInput,
        DescribeTimeToLiveInput, GetItemInput, ListTablesInput, ListTagsOfResourceInput,
        PutItemInput, QueryInput, ScanInput, TagResourceInput, TransactGetItemsInput,
        TransactWriteItemsInput, UntagResourceInput, UpdateItemInput, UpdateTableInput,
        UpdateTimeToLiveInput,
    },
    output::{
        BatchGetItemOutput, BatchWriteItemOutput, CreateTableOutput, DeleteItemOutput,
        DeleteTableOutput, DescribeEndpointsOutput, DescribeLimitsOutput, DescribeTableOutput,
        DescribeTimeToLiveOutput, Endpoint, GetItemOutput, ListTablesOutput,
        ListTagsOfResourceOutput, PutItemOutput, QueryOutput, ScanOutput, TagResourceOutput,
        TransactGetItemsOutput, TransactWriteItemsOutput, UntagResourceOutput, UpdateItemOutput,
        UpdateTableOutput, UpdateTimeToLiveOutput,
    },
    types::{
        AttributeAction, AttributeDefinition, AttributeValueUpdate, BillingMode,
        CancellationReason, ComparisonOperator, Condition, ConditionalOperator,
        ExpectedAttributeValue, ItemResponse, KeyType, ReturnValue, ScalarAttributeType, Select,
        TableStatus, TimeToLiveDescription,
    },
};

use crate::{
    config::DynamoDBConfig,
    error::{expression_error_to_dynamodb, storage_error_to_dynamodb},
    expression::{
        AttributePath, EvalContext, PathElement, UpdateExpr, collect_names_from_expr,
        collect_names_from_projection, collect_names_from_update, collect_paths_from_expr,
        collect_values_from_expr, collect_values_from_update, parse_condition, parse_projection,
        parse_update,
    },
    state::{DynamoDBServiceState, DynamoDBTable},
    storage::{
        KeyAttribute, KeySchema, PrimaryKey, SortKeyCondition, SortableAttributeValue,
        TableStorage, calculate_item_size, extract_primary_key, partition_key_segment,
    },
};

/// Maximum item size in bytes (400 KB).
const MAX_ITEM_SIZE_BYTES: u64 = 400 * 1024;

/// Maximum number of significant digits allowed for DynamoDB numbers.
const MAX_SIGNIFICANT_DIGITS: usize = 38;

/// Validate a DynamoDB number string for format, magnitude, and precision.
///
/// Returns `Ok(())` if the number is valid, or an error with an appropriate
/// message for invalid format, overflow, underflow, or precision violations.
fn validate_number_string(s: &str) -> Result<(), DynamoDBError> {
    // Reject leading/trailing spaces.
    if s != s.trim() {
        return Err(DynamoDBError::validation(
            "The parameter cannot be converted to a numeric value: numeric value is not valid",
        ));
    }
    let trimmed = s.trim();

    // Reject empty strings.
    if trimmed.is_empty() {
        return Err(DynamoDBError::validation(
            "The parameter cannot be converted to a numeric value",
        ));
    }

    // Reject NaN, Infinity, and other non-numeric strings.
    // Only allow: optional sign, digits, optional decimal point, optional exponent.
    let rest = trimmed.strip_prefix(['+', '-']).unwrap_or(trimmed);
    if rest.is_empty() {
        return Err(DynamoDBError::validation(
            "The parameter cannot be converted to a numeric value: numeric value is not valid",
        ));
    }

    // Split into mantissa and exponent parts.
    let (mantissa, explicit_exp) = if let Some(pos) = rest.find(['e', 'E']) {
        let exp_str = &rest[pos + 1..];
        let exp: i64 = exp_str.parse().map_err(|_| {
            DynamoDBError::validation(
                "The parameter cannot be converted to a numeric value: numeric value is not valid",
            )
        })?;
        (&rest[..pos], exp)
    } else {
        (rest, 0i64)
    };

    // Validate mantissa: must be digits with optional single decimal point.
    // Reject leading/trailing spaces (already trimmed), but also reject
    // mantissa that has no digits at all.
    if mantissa.is_empty() {
        return Err(DynamoDBError::validation(
            "The parameter cannot be converted to a numeric value: numeric value is not valid",
        ));
    }

    let mut has_dot = false;
    let mut has_digit = false;
    for ch in mantissa.chars() {
        if ch == '.' {
            if has_dot {
                return Err(DynamoDBError::validation(
                    "The parameter cannot be converted to a numeric value: numeric value is not \
                     valid",
                ));
            }
            has_dot = true;
        } else if ch.is_ascii_digit() {
            has_digit = true;
        } else {
            return Err(DynamoDBError::validation(
                "The parameter cannot be converted to a numeric value: numeric value is not valid",
            ));
        }
    }
    if !has_digit {
        return Err(DynamoDBError::validation(
            "The parameter cannot be converted to a numeric value: numeric value is not valid",
        ));
    }

    // Remove the decimal point and compute significant digits.
    let all_digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let trimmed_leading = all_digits.trim_start_matches('0');
    let significant = trimmed_leading.trim_end_matches('0');

    // Check if the number is zero (all zeros).
    if significant.is_empty() {
        // Zero is always valid regardless of exponent.
        return Ok(());
    }

    // Validate precision: max 38 significant digits.
    let sig_digit_count = trimmed_leading.trim_end_matches('0').len();
    if sig_digit_count > MAX_SIGNIFICANT_DIGITS {
        return Err(DynamoDBError::validation(format!(
            "Attempting to store more than {MAX_SIGNIFICANT_DIGITS} significant digits in a Number"
        )));
    }

    // Compute the actual magnitude of the number.
    // The number is: significant_digits * 10^(explicit_exp - frac_digits +
    // trailing_zeros_in_significant)
    let dot_pos = mantissa.find('.');
    #[allow(clippy::cast_possible_wrap)]
    let frac_digits = if let Some(pos) = dot_pos {
        (mantissa.len() - pos - 1) as i64
    } else {
        0i64
    };
    // Number of leading zeros in all_digits
    #[allow(clippy::cast_possible_wrap)]
    let leading_zeros = (all_digits.len() - trimmed_leading.len()) as i64;
    // Actual magnitude = explicit_exp - frac_digits + all_digits.len() - leading_zeros - 1
    #[allow(clippy::cast_possible_wrap)]
    let magnitude = explicit_exp - frac_digits + all_digits.len() as i64 - leading_zeros - 1;

    if magnitude > 125 {
        return Err(DynamoDBError::validation(
            "Number overflow. Attempting to store a number with magnitude larger than supported \
             range",
        ));
    }
    // The smallest allowed magnitude is -130.
    // A number like 1e-130 has magnitude = -130, which is allowed.
    // A number like 1e-131 has magnitude = -131, which is NOT allowed.
    if magnitude < -130 {
        return Err(DynamoDBError::validation(
            "Number underflow. Attempting to store a number with magnitude smaller than supported \
             range",
        ));
    }

    Ok(())
}

/// Validate all number-type values in an attribute value map.
fn validate_numbers_in_item(item: &HashMap<String, AttributeValue>) -> Result<(), DynamoDBError> {
    for val in item.values() {
        validate_numbers_in_value(val)?;
    }
    Ok(())
}

/// Recursively validate number-type values in an `AttributeValue`.
fn validate_numbers_in_value(val: &AttributeValue) -> Result<(), DynamoDBError> {
    match val {
        AttributeValue::N(s) => validate_number_string(s),
        AttributeValue::Ns(nums) => {
            for n in nums {
                validate_number_string(n)?;
            }
            Ok(())
        }
        AttributeValue::L(list) => {
            for v in list {
                validate_numbers_in_value(v)?;
            }
            Ok(())
        }
        AttributeValue::M(map) => {
            for v in map.values() {
                validate_numbers_in_value(v)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate a table name against DynamoDB rules: 3-255 characters, `[a-zA-Z0-9._-]+`.
fn validate_table_name(name: &str) -> Result<(), DynamoDBError> {
    if name.len() < 3 || name.len() > 255 {
        return Err(DynamoDBError::validation(format!(
            "TableName must be at least 3 characters long and at most 255 characters long, but \
             was {} characters",
            name.len()
        )));
    }
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(DynamoDBError::validation(format!(
            "1 validation error detected: Value '{name}' at 'tableName' failed to satisfy \
             constraint: Member must satisfy regular expression pattern: [a-zA-Z0-9_.-]+"
        )));
    }
    Ok(())
}

/// Validate that key attributes in the given item do not contain empty string/binary values.
fn validate_key_not_empty(
    key_schema: &KeySchema,
    item: &HashMap<String, AttributeValue>,
) -> Result<(), DynamoDBError> {
    for ka in std::iter::once(&key_schema.partition_key).chain(key_schema.sort_key.iter()) {
        if let Some(val) = item.get(&ka.name) {
            match val {
                AttributeValue::S(s) if s.is_empty() => {
                    return Err(DynamoDBError::validation(format!(
                        "One or more parameter values are not valid. The AttributeValue for a key \
                         attribute cannot contain an empty string value. Key: {}",
                        ka.name
                    )));
                }
                AttributeValue::B(b) if b.is_empty() => {
                    return Err(DynamoDBError::validation(format!(
                        "One or more parameter values are not valid. The AttributeValue for a key \
                         attribute cannot contain an empty string value. Key: {}",
                        ka.name
                    )));
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Validate that the `Key` map only contains the key attributes and nothing else.
fn validate_key_only_has_key_attrs(
    key_schema: &KeySchema,
    key: &HashMap<String, AttributeValue>,
) -> Result<(), DynamoDBError> {
    for attr_name in key.keys() {
        if !is_key_attribute(attr_name, key_schema) {
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values are not valid. Number of user supplied keys don't \
                 match number of table schema keys. Keys provided: [{}], schema keys: [{}]",
                format_key_names(key),
                format_schema_key_names(key_schema),
            )));
        }
    }
    Ok(())
}

/// Validate that key attribute values match the expected types from the key schema.
///
/// DynamoDB only allows S, N, or B for key attributes. This function checks
/// that each provided key value has the type matching the schema definition.
fn validate_key_types(
    key_schema: &KeySchema,
    key: &HashMap<String, AttributeValue>,
) -> Result<(), DynamoDBError> {
    for ka in std::iter::once(&key_schema.partition_key).chain(key_schema.sort_key.iter()) {
        if let Some(val) = key.get(&ka.name) {
            let type_matches = match &ka.attr_type {
                ScalarAttributeType::S => matches!(val, AttributeValue::S(_)),
                ScalarAttributeType::N => matches!(val, AttributeValue::N(_)),
                ScalarAttributeType::B => matches!(val, AttributeValue::B(_)),
                ScalarAttributeType::Unknown(_) => false,
            };
            if !type_matches {
                return Err(DynamoDBError::validation(format!(
                    "The provided key element does not match the schema. Expected type {expected} \
                     for key column {name}, got type {actual}",
                    expected = ka.attr_type,
                    name = ka.name,
                    actual = val.type_descriptor(),
                )));
            }
        }
    }
    Ok(())
}

/// Validate that `AttributesToGet` does not contain duplicate attribute names.
fn validate_no_duplicate_attributes_to_get(attrs: &[String]) -> Result<(), DynamoDBError> {
    let mut seen = HashSet::new();
    for attr in attrs {
        if !seen.insert(attr.as_str()) {
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values are not valid. Duplicate value in AttributesToGet: \
                 {attr}"
            )));
        }
    }
    Ok(())
}

/// Wrap an expression error with ProjectionExpression context.
#[allow(clippy::needless_pass_by_value)]
fn projection_error_to_dynamodb(e: crate::expression::ExpressionError) -> DynamoDBError {
    DynamoDBError::validation(format!("Invalid ProjectionExpression: {e}"))
}

/// Format key attribute names from a key map for error messages.
fn format_key_names(key: &HashMap<String, AttributeValue>) -> String {
    let mut names: Vec<&str> = key.keys().map(String::as_str).collect();
    names.sort_unstable();
    names.join(", ")
}

/// Format key schema attribute names for error messages.
fn format_schema_key_names(key_schema: &KeySchema) -> String {
    let mut names = vec![key_schema.partition_key.name.as_str()];
    if let Some(ref sk) = key_schema.sort_key {
        names.push(sk.name.as_str());
    }
    names.join(", ")
}

/// Validate the `Select` parameter for Query/Scan, checking conflicts with
/// `ProjectionExpression` and `AttributesToGet`.
fn validate_select(
    select: Option<&Select>,
    has_projection: bool,
    has_attributes_to_get: bool,
) -> Result<(), DynamoDBError> {
    if let Some(sel) = select {
        match sel {
            Select::AllProjectedAttributes => {
                return Err(DynamoDBError::validation(
                    "ALL_PROJECTED_ATTRIBUTES is only supported for queries on secondary indexes",
                ));
            }
            Select::SpecificAttributes => {
                if !has_projection && !has_attributes_to_get {
                    return Err(DynamoDBError::validation(
                        "SPECIFIC_ATTRIBUTES requires either ProjectionExpression or \
                         AttributesToGet",
                    ));
                }
            }
            Select::AllAttributes | Select::Count => {
                if has_attributes_to_get {
                    return Err(DynamoDBError::validation(format!(
                        "Cannot specify the AttributesToGet when choosing to get {} results",
                        sel.as_str()
                    )));
                }
                if has_projection {
                    return Err(DynamoDBError::validation(format!(
                        "Cannot specify the ProjectionExpression when choosing to get {} results",
                        sel.as_str()
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Maximum allowed value for `TotalSegments` in a parallel scan.
const MAX_TOTAL_SEGMENTS: i32 = 1_000_000;

/// Validate and extract parallel scan parameters (`Segment` / `TotalSegments`).
///
/// Returns `(Some(segment), Some(total_segments))` when parallel scan is
/// requested, or `(None, None)` when it is not.
fn validate_parallel_scan(
    input: &ScanInput,
    exclusive_start_key: Option<&PrimaryKey>,
) -> Result<(Option<u32>, Option<u32>), DynamoDBError> {
    match (input.segment, input.total_segments) {
        (Some(seg), Some(total)) => {
            // TotalSegments must be in [1, MAX_TOTAL_SEGMENTS].
            if total > MAX_TOTAL_SEGMENTS {
                return Err(DynamoDBError::validation(format!(
                    "1 validation error detected: Value '{total}' at 'totalSegments' failed to \
                     satisfy constraint: Member must have value less than or equal to \
                     {MAX_TOTAL_SEGMENTS}. The Segment parameter is required but was not present \
                     in the request when parameter TotalSegments is present"
                )));
            }
            // Segment must be in [0, TotalSegments).
            if seg >= total {
                return Err(DynamoDBError::validation(format!(
                    "The Segment parameter is zero-indexed and must be less than parameter \
                     TotalSegments. Segment: {seg}, TotalSegments: {total}"
                )));
            }
            // ExclusiveStartKey must map to the same segment.
            if let Some(start_key) = exclusive_start_key {
                #[allow(clippy::cast_sign_loss)] // Validated above
                let key_segment = partition_key_segment(&start_key.partition_key, total as u32);
                #[allow(clippy::cast_sign_loss)]
                if key_segment != seg as u32 {
                    return Err(DynamoDBError::validation(
                        "The provided Exclusive start key does not map to the provided Segment \
                         and TotalSegments values."
                            .to_owned(),
                    ));
                }
            }
            #[allow(clippy::cast_sign_loss)] // Validated: seg >= 0, total >= 1
            Ok((Some(seg as u32), Some(total as u32)))
        }
        (None, None) => Ok((None, None)),
        // If only one is provided, DynamoDB returns an error about the
        // missing one, but boto3 rejects this client-side. We still
        // handle it for raw API callers.
        (Some(_), None) => Err(DynamoDBError::validation(
            "The TotalSegments parameter is required but was not present in the request when \
             parameter Segment is present",
        )),
        (None, Some(_)) => Err(DynamoDBError::validation(
            "The Segment parameter is required but was not present in the request when parameter \
             TotalSegments is present",
        )),
    }
}

/// Main DynamoDB provider implementing all operations.
pub struct RustStackDynamoDB {
    /// Service state owning all tables.
    pub state: Arc<DynamoDBServiceState>,
    /// Configuration.
    pub config: Arc<DynamoDBConfig>,
    /// Stream emitter for change data capture.
    emitter: Arc<dyn crate::stream::StreamEmitter>,
    /// Stream lifecycle manager.
    lifecycle: Arc<dyn crate::stream::StreamLifecycle>,
}

impl std::fmt::Debug for RustStackDynamoDB {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RustStackDynamoDB")
            .field("state", &self.state)
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl RustStackDynamoDB {
    /// Create a new DynamoDB provider.
    #[must_use]
    pub fn new(config: DynamoDBConfig) -> Self {
        Self {
            state: Arc::new(DynamoDBServiceState::new()),
            config: Arc::new(config),
            emitter: Arc::new(crate::stream::NoopStreamEmitter),
            lifecycle: Arc::new(crate::stream::NoopStreamLifecycle),
        }
    }

    /// Set the stream emitter for change data capture.
    ///
    /// Called by the server binary to wire in the DynamoDB Streams
    /// implementation.
    pub fn set_emitter(&mut self, emitter: Arc<dyn crate::stream::StreamEmitter>) {
        self.emitter = emitter;
    }

    /// Set the stream lifecycle manager.
    ///
    /// Called by the server binary to wire in stream creation/deletion.
    pub fn set_lifecycle(&mut self, lifecycle: Arc<dyn crate::stream::StreamLifecycle>) {
        self.lifecycle = lifecycle;
    }

    /// Reset all state (for testing).
    pub fn reset(&self) {
        self.state.reset();
    }
}

// ---------------------------------------------------------------------------
// Table management
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `CreateTable`.
    #[allow(clippy::too_many_lines)]
    pub fn handle_create_table(
        &self,
        input: CreateTableInput,
    ) -> Result<CreateTableOutput, DynamoDBError> {
        // Validate table name.
        validate_table_name(&input.table_name)?;

        // Validate attribute definitions are present.
        if input.attribute_definitions.is_empty() {
            return Err(DynamoDBError::validation(
                "One or more parameter values were invalid: Some AttributeDefinitions are not \
                 valid. AttributeDefinitions must be provided for all key attributes",
            ));
        }

        // Validate attribute definitions: no duplicate attribute names.
        validate_attribute_definitions(&input.attribute_definitions)?;

        // Validate key schema structure: exactly 1 HASH, at most 1 RANGE.
        validate_key_schema_structure(&input.key_schema)?;

        // Validate key schema: all key attributes defined in AttributeDefinitions,
        // types are valid for keys (S, N, B only).
        let key_schema = parse_key_schema(&input.key_schema, &input.attribute_definitions)?;

        // Validate billing mode.
        let billing = validate_billing_mode(
            input.billing_mode.as_ref(),
            input.provisioned_throughput.as_ref(),
        )?;

        // Validate no spurious attribute definitions: all defined attributes
        // must be used as a key in the table or an index.
        validate_no_spurious_attribute_definitions(
            &input.attribute_definitions,
            &input.key_schema,
            &input.global_secondary_indexes,
            &input.local_secondary_indexes,
        )?;

        // Build storage.
        let storage = TableStorage::new(key_schema.clone());

        let table_name = input.table_name.clone();
        let arn = format!(
            "arn:aws:dynamodb:{}:000000000000:table/{}",
            self.config.default_region, table_name,
        );

        let table = DynamoDBTable {
            name: table_name,
            status: TableStatus::Active,
            key_schema_elements: input.key_schema,
            key_schema,
            attribute_definitions: input.attribute_definitions,
            billing_mode: billing,
            provisioned_throughput: input.provisioned_throughput,
            gsi_definitions: input.global_secondary_indexes,
            lsi_definitions: input.local_secondary_indexes,
            stream_specification: input.stream_specification,
            sse_specification: input.sse_specification,
            tags: parking_lot::RwLock::new(input.tags),
            ttl: parking_lot::RwLock::new(None),
            arn,
            table_id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now(),
            storage,
        };

        let table = self.state.create_table(table)?;

        // Create stream if StreamSpecification is enabled.
        let mut desc = table.to_description();
        if let Some(ref spec) = table.stream_specification {
            if spec.stream_enabled {
                let svt = spec
                    .stream_view_type
                    .clone()
                    .unwrap_or(ruststack_dynamodb_model::types::StreamViewType::NewAndOldImages);
                let stream_arn = self.lifecycle.on_stream_enabled(
                    &table.name,
                    &table.arn,
                    table.key_schema_elements.clone(),
                    svt,
                );
                if !stream_arn.is_empty() {
                    let stream_label = self.lifecycle.get_stream_label(&table.name);
                    desc.latest_stream_arn = Some(stream_arn);
                    desc.latest_stream_label = stream_label;
                }
            }
        }

        Ok(CreateTableOutput {
            table_description: Some(desc),
        })
    }

    /// Handle `DeleteTable`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_delete_table(
        &self,
        input: DeleteTableInput,
    ) -> Result<DeleteTableOutput, DynamoDBError> {
        let table = self.state.delete_table(&input.table_name)?;
        self.lifecycle.on_table_deleted(&table.name);
        Ok(DeleteTableOutput {
            table_description: Some(table.to_delete_description()),
        })
    }

    /// Handle `DescribeTable`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_describe_table(
        &self,
        input: DescribeTableInput,
    ) -> Result<DescribeTableOutput, DynamoDBError> {
        let table = self.state.require_table(&input.table_name)?;
        let mut desc = table.to_description();

        // Add stream ARN/label if stream is enabled.
        if let Some(ref spec) = table.stream_specification {
            if spec.stream_enabled {
                desc.latest_stream_arn = self.lifecycle.get_stream_arn(&table.name);
                desc.latest_stream_label = self.lifecycle.get_stream_label(&table.name);
            }
        }

        Ok(DescribeTableOutput { table: Some(desc) })
    }

    /// Handle `ListTables`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_list_tables(
        &self,
        input: ListTablesInput,
    ) -> Result<ListTablesOutput, DynamoDBError> {
        // Validate limit: must be 1-100 if specified.
        if let Some(limit) = input.limit {
            if !(1..=100).contains(&limit) {
                return Err(DynamoDBError::validation(format!(
                    "1 validation error detected: Value '{limit}' at 'limit' failed to satisfy \
                     constraint: Member must have value less than or equal to 100"
                )));
            }
        }

        let all_names = self.state.list_table_names();
        let limit = usize::try_from(input.limit.unwrap_or(100).clamp(1, 100)).unwrap_or(100);

        let start_idx = if let Some(ref start) = input.exclusive_start_table_name {
            all_names
                .iter()
                .position(|n| n.as_str() > start.as_str())
                .unwrap_or(all_names.len())
        } else {
            0
        };

        let page: Vec<String> = all_names
            .into_iter()
            .skip(start_idx)
            .take(limit + 1)
            .collect();

        let (table_names, last) = if page.len() > limit {
            let last = page[limit - 1].clone();
            (page[..limit].to_vec(), Some(last))
        } else {
            (page, None)
        };

        Ok(ListTablesOutput {
            table_names,
            last_evaluated_table_name: last,
        })
    }

    /// Handle `UpdateTable`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_update_table(
        &self,
        input: UpdateTableInput,
    ) -> Result<UpdateTableOutput, DynamoDBError> {
        let table = self.state.require_table(&input.table_name)?;

        // For our in-memory emulator, UpdateTable is accepted but most changes
        // are not enforced (billing mode, provisioned throughput, attribute
        // definitions are metadata-only in DynamoDB local emulation). We return
        // the current table description with ACTIVE status, which matches
        // LocalStack behaviour.
        let desc = table.to_description();
        Ok(UpdateTableOutput {
            table_description: Some(desc),
        })
    }
}

// ---------------------------------------------------------------------------
// Item CRUD
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `PutItem`.
    #[allow(clippy::too_many_lines)]
    pub fn handle_put_item(&self, mut input: PutItemInput) -> Result<PutItemOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        // Validate return_values: PutItem only supports NONE and ALL_OLD.
        if let Some(ref rv) = input.return_values {
            if !matches!(rv, ReturnValue::None | ReturnValue::AllOld) {
                return Err(DynamoDBError::validation(format!(
                    "Return values set to invalid value for this operation: {rv}"
                )));
            }
        }

        // Validate key attributes are not empty.
        validate_key_not_empty(&table.key_schema, &input.item)?;

        // Validate item does not contain empty sets.
        validate_item_no_empty_sets(&input.item)?;

        // Reject mixing Expected with ConditionExpression.
        if !input.expected.is_empty() && input.condition_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {Expected} Expression parameters: \
                 {ConditionExpression}",
            ));
        }

        // Legacy API: convert Expected to ConditionExpression.
        if !input.expected.is_empty() && input.condition_expression.is_none() {
            validate_expected(&input.expected)?;
            let (expr, names, values) =
                convert_expected_to_condition(&input.expected, input.conditional_operator.as_ref());
            input.condition_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Validate return_values_on_condition_check_failure.
        validate_return_values_on_condition_check_failure(
            input.return_values_on_condition_check_failure.as_deref(),
        )?;

        // Validate numbers in item.
        validate_numbers_in_item(&input.item)?;

        // Validate item size.
        let size = calculate_item_size(&input.item);
        if size > MAX_ITEM_SIZE_BYTES {
            return Err(DynamoDBError::validation(format!(
                "Item size has exceeded the maximum allowed size of {MAX_ITEM_SIZE_BYTES} bytes"
            )));
        }

        // Reject empty condition expression.
        validate_condition_not_empty(input.condition_expression.as_deref())?;

        // Validate unused expression attribute names/values.
        {
            let mut used_names = HashSet::new();
            let mut used_values = HashSet::new();
            if let Some(ref condition) = input.condition_expression {
                let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&expr, &mut used_names);
                collect_values_from_expr(&expr, &mut used_values);
            }
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_values(&input.expression_attribute_values, &used_values)?;
        }

        // Evaluate condition expression if present.
        if let Some(ref condition) = input.condition_expression {
            let existing = {
                let pk = extract_primary_key(&table.key_schema, &input.item)
                    .map_err(storage_error_to_dynamodb)?;
                table.storage.get_item(&pk)
            };
            let empty = HashMap::new();
            let item_ref = existing.as_ref().unwrap_or(&empty);
            let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
            let ctx = EvalContext {
                item: item_ref,
                names: &input.expression_attribute_names,
                values: &input.expression_attribute_values,
            };
            let result = ctx.evaluate(&expr).map_err(expression_error_to_dynamodb)?;
            if !result {
                let mut err =
                    DynamoDBError::conditional_check_failed("The conditional request failed");
                if input.return_values_on_condition_check_failure.as_deref() == Some("ALL_OLD") {
                    if let Some(ref existing_item) = existing {
                        err = err.with_item(existing_item.clone());
                    }
                }
                return Err(err);
            }
        }

        let new_item = input.item.clone();
        let old = table
            .storage
            .put_item(input.item)
            .map_err(storage_error_to_dynamodb)?;

        // Emit stream event if stream is enabled for this table.
        if table
            .stream_specification
            .as_ref()
            .is_some_and(|s| s.stream_enabled)
        {
            let event_name = if old.is_some() {
                crate::stream::ChangeEventName::Modify
            } else {
                crate::stream::ChangeEventName::Insert
            };
            let keys = extract_key_attributes(&new_item, &table.key_schema_elements);
            let size = calculate_item_size(&new_item);
            self.emitter.emit(crate::stream::ChangeEvent {
                table_name: table.name.clone(),
                event_name,
                keys,
                old_image: old.clone(),
                new_image: Some(new_item),
                size_bytes: size,
            });
        }

        // ALL_OLD: return the old item if it existed, otherwise omit Attributes.
        let attributes = match input.return_values {
            Some(ReturnValue::AllOld) => old.unwrap_or_default(),
            _ => HashMap::new(),
        };

        Ok(PutItemOutput {
            attributes,
            consumed_capacity: None,
            item_collection_metrics: None,
        })
    }

    /// Handle `GetItem`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_get_item(&self, mut input: GetItemInput) -> Result<GetItemOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        // Reject both ProjectionExpression and AttributesToGet.
        if input.projection_expression.is_some()
            && input
                .attributes_to_get
                .as_ref()
                .is_some_and(|v| !v.is_empty())
        {
            return Err(DynamoDBError::validation(
                "Cannot have both AttributesToGet and ProjectionExpression",
            ));
        }

        // Validate key: no spurious columns, correct types, not empty.
        validate_key_only_has_key_attrs(&table.key_schema, &input.key)?;
        validate_key_types(&table.key_schema, &input.key)?;
        validate_key_not_empty(&table.key_schema, &input.key)?;
        let pk = extract_primary_key(&table.key_schema, &input.key)
            .map_err(storage_error_to_dynamodb)?;

        // Legacy API: convert AttributesToGet to ProjectionExpression.
        if let Some(ref atg) = input.attributes_to_get {
            if atg.is_empty() {
                return Err(DynamoDBError::validation(
                    "One or more parameter values are not valid. The AttributesToGet parameter \
                     must contain at least one element",
                ));
            }
            validate_no_duplicate_attributes_to_get(atg)?;
            if input.projection_expression.is_none() {
                input.projection_expression = Some(convert_attributes_to_get(atg));
            }
        }

        // Validate unused expression attribute names.
        {
            let mut used_names = HashSet::new();
            if let Some(ref proj) = input.projection_expression {
                let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
                collect_names_from_projection(&paths, &mut used_names);
            }
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
        }

        let item = table.storage.get_item(&pk);

        // Apply projection if specified.
        let projected = match (item, &input.projection_expression) {
            (Some(item), Some(proj_expr)) => {
                let paths = parse_projection(proj_expr).map_err(projection_error_to_dynamodb)?;
                let ctx = EvalContext {
                    item: &item,
                    names: &input.expression_attribute_names,
                    values: &HashMap::new(),
                };
                Some(ctx.apply_projection(&paths))
            }
            (Some(item), None) => Some(item),
            (None, _) => None,
        };

        Ok(GetItemOutput {
            item: projected,
            consumed_capacity: None,
        })
    }

    /// Handle `DeleteItem`.
    pub fn handle_delete_item(
        &self,
        mut input: DeleteItemInput,
    ) -> Result<DeleteItemOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        // Validate return_values: DeleteItem only supports NONE and ALL_OLD.
        if let Some(ref rv) = input.return_values {
            if !matches!(rv, ReturnValue::None | ReturnValue::AllOld) {
                return Err(DynamoDBError::validation(format!(
                    "Return values set to invalid value for this operation: {rv}"
                )));
            }
        }

        // Validate return_values_on_condition_check_failure.
        validate_return_values_on_condition_check_failure(
            input.return_values_on_condition_check_failure.as_deref(),
        )?;

        // Validate key: no spurious columns, correct types, not empty.
        validate_key_only_has_key_attrs(&table.key_schema, &input.key)?;
        validate_key_not_empty(&table.key_schema, &input.key)?;
        let pk = extract_primary_key(&table.key_schema, &input.key)
            .map_err(storage_error_to_dynamodb)?;

        // Validate ConditionalOperator usage.
        validate_conditional_operator(input.conditional_operator.as_ref(), &input.expected)?;

        // Reject mixing Expected with ConditionExpression.
        if !input.expected.is_empty() && input.condition_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {Expected} Expression parameters: \
                 {ConditionExpression}",
            ));
        }

        // Legacy API: convert Expected to ConditionExpression.
        if !input.expected.is_empty() && input.condition_expression.is_none() {
            validate_expected(&input.expected)?;
            let (expr, names, values) =
                convert_expected_to_condition(&input.expected, input.conditional_operator.as_ref());
            input.condition_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Reject empty condition expression.
        validate_condition_not_empty(input.condition_expression.as_deref())?;

        // Validate unused expression attribute names/values.
        {
            let mut used_names = HashSet::new();
            let mut used_values = HashSet::new();
            if let Some(ref condition) = input.condition_expression {
                let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&expr, &mut used_names);
                collect_values_from_expr(&expr, &mut used_values);
            }
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_values(&input.expression_attribute_values, &used_values)?;
        }

        // Evaluate condition expression if present.
        if let Some(ref condition) = input.condition_expression {
            let existing = table.storage.get_item(&pk);
            let empty = HashMap::new();
            let item_ref = existing.as_ref().unwrap_or(&empty);
            let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
            let ctx = EvalContext {
                item: item_ref,
                names: &input.expression_attribute_names,
                values: &input.expression_attribute_values,
            };
            let result = ctx.evaluate(&expr).map_err(expression_error_to_dynamodb)?;
            if !result {
                let mut err =
                    DynamoDBError::conditional_check_failed("The conditional request failed");
                if input.return_values_on_condition_check_failure.as_deref() == Some("ALL_OLD") {
                    if let Some(ref existing_item) = existing {
                        err = err.with_item(existing_item.clone());
                    }
                }
                return Err(err);
            }
        }

        let old = table.storage.delete_item(&pk);

        // Emit stream event if an item was actually deleted and stream is enabled.
        if let Some(ref old_item) = old {
            if table
                .stream_specification
                .as_ref()
                .is_some_and(|s| s.stream_enabled)
            {
                let keys = extract_key_attributes(old_item, &table.key_schema_elements);
                let size = calculate_item_size(old_item);
                self.emitter.emit(crate::stream::ChangeEvent {
                    table_name: table.name.clone(),
                    event_name: crate::stream::ChangeEventName::Remove,
                    keys,
                    old_image: Some(old_item.clone()),
                    new_image: None,
                    size_bytes: size,
                });
            }
        }

        // ALL_OLD: return the old item if it existed, otherwise omit Attributes.
        let attributes = match input.return_values {
            Some(ReturnValue::AllOld) => old.unwrap_or_default(),
            _ => HashMap::new(),
        };

        Ok(DeleteItemOutput {
            attributes,
            consumed_capacity: None,
            item_collection_metrics: None,
        })
    }

    /// Handle `UpdateItem`.
    #[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
    pub fn handle_update_item(
        &self,
        mut input: UpdateItemInput,
    ) -> Result<UpdateItemOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        // Validate return_values.
        if let Some(ref rv) = input.return_values {
            if !matches!(
                rv,
                ReturnValue::None
                    | ReturnValue::AllOld
                    | ReturnValue::AllNew
                    | ReturnValue::UpdatedOld
                    | ReturnValue::UpdatedNew
            ) {
                return Err(DynamoDBError::validation(format!(
                    "Return values set to invalid value for this operation: {rv}"
                )));
            }
        }

        // Validate return_values_on_condition_check_failure.
        validate_return_values_on_condition_check_failure(
            input.return_values_on_condition_check_failure.as_deref(),
        )?;

        // Validate ConditionalOperator usage.
        validate_conditional_operator(input.conditional_operator.as_ref(), &input.expected)?;

        // Reject mixing non-expression parameters with expression parameters.
        if !input.attribute_updates.is_empty() && input.update_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {AttributeUpdates} Expression parameters: \
                 {UpdateExpression}",
            ));
        }
        if !input.attribute_updates.is_empty() && input.condition_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {AttributeUpdates} Expression parameters: \
                 {ConditionExpression}",
            ));
        }
        if !input.expected.is_empty() && input.update_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {Expected} Expression parameters: {UpdateExpression}",
            ));
        }
        if !input.expected.is_empty() && input.condition_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {Expected} Expression parameters: \
                 {ConditionExpression}",
            ));
        }

        // Validate key: no spurious columns, correct types, not empty.
        validate_key_only_has_key_attrs(&table.key_schema, &input.key)?;
        validate_key_not_empty(&table.key_schema, &input.key)?;
        validate_numbers_in_item(&input.key)?;
        let pk = extract_primary_key(&table.key_schema, &input.key)
            .map_err(storage_error_to_dynamodb)?;

        let existing = table.storage.get_item(&pk);
        let mut item = existing.clone().unwrap_or_else(|| input.key.clone());

        // Legacy API: convert Expected to ConditionExpression.
        if !input.expected.is_empty() && input.condition_expression.is_none() {
            validate_expected(&input.expected)?;
            let (expr, names, values) =
                convert_expected_to_condition(&input.expected, input.conditional_operator.as_ref());
            input.condition_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Legacy API: handle ADD-for-lists by applying list_append directly.
        // The modern UpdateExpression ADD does not support lists, but the
        // legacy AttributeUpdates ADD does (it appends to lists).
        if !input.attribute_updates.is_empty() && input.update_expression.is_none() {
            let list_add_attrs: Vec<String> = input
                .attribute_updates
                .iter()
                .filter(|(_, u)| {
                    u.action
                        .as_ref()
                        .is_some_and(|a| *a == AttributeAction::Add)
                        && u.value
                            .as_ref()
                            .is_some_and(|v| matches!(v, AttributeValue::L(_)))
                })
                .map(|(k, _)| k.clone())
                .collect();
            for attr in &list_add_attrs {
                if let Some(update) = input.attribute_updates.remove(attr) {
                    if let Some(AttributeValue::L(new_items)) = update.value {
                        match item.get(attr) {
                            Some(AttributeValue::L(existing)) => {
                                let mut merged = existing.clone();
                                merged.extend(new_items);
                                item.insert(attr.clone(), AttributeValue::L(merged));
                            }
                            None => {
                                item.insert(attr.clone(), AttributeValue::L(new_items));
                            }
                            Some(existing_val) => {
                                return Err(DynamoDBError::validation(format!(
                                    "Type mismatch for ADD; operator type: L, existing type: {}",
                                    existing_val.type_descriptor(),
                                )));
                            }
                        }
                    }
                }
            }
        }

        // Legacy API: convert remaining AttributeUpdates to UpdateExpression.
        if !input.attribute_updates.is_empty() && input.update_expression.is_none() {
            let (expr, names, values) =
                convert_attribute_updates_to_expression(&input.attribute_updates);
            input.update_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Validate numbers in expression attribute values.
        validate_numbers_in_item(&input.expression_attribute_values)?;

        // Validate expression attribute values contain no empty sets.
        validate_no_empty_sets(&input.expression_attribute_values)?;

        // Reject empty condition expression.
        validate_condition_not_empty(input.condition_expression.as_deref())?;

        // Validate unused expression attribute names/values.
        {
            let mut used_names = HashSet::new();
            let mut used_values = HashSet::new();
            if let Some(ref condition) = input.condition_expression {
                let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&expr, &mut used_names);
                collect_values_from_expr(&expr, &mut used_values);
            }
            if let Some(ref update_expr) = input.update_expression {
                let parsed = parse_update(update_expr).map_err(expression_error_to_dynamodb)?;
                collect_names_from_update(&parsed, &mut used_names);
                collect_values_from_update(&parsed, &mut used_values);
            }
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_values(&input.expression_attribute_values, &used_values)?;
        }

        // Validate update expression: key attributes cannot be modified,
        // and paths must not overlap.
        if let Some(ref update_expr) = input.update_expression {
            let parsed = parse_update(update_expr).map_err(expression_error_to_dynamodb)?;
            validate_update_paths(
                &parsed,
                &table.key_schema,
                &input.expression_attribute_names,
            )?;
        }

        // Evaluate condition expression.
        if let Some(ref condition) = input.condition_expression {
            let empty = HashMap::new();
            let item_ref = existing.as_ref().unwrap_or(&empty);
            let expr = parse_condition(condition).map_err(expression_error_to_dynamodb)?;
            let ctx = EvalContext {
                item: item_ref,
                names: &input.expression_attribute_names,
                values: &input.expression_attribute_values,
            };
            let result = ctx.evaluate(&expr).map_err(expression_error_to_dynamodb)?;
            if !result {
                let mut err =
                    DynamoDBError::conditional_check_failed("The conditional request failed");
                if input.return_values_on_condition_check_failure.as_deref() == Some("ALL_OLD") {
                    let existing_ref = existing.as_ref();
                    if let Some(existing_item) = existing_ref {
                        err = err.with_item(existing_item.clone());
                    }
                }
                return Err(err);
            }
        }

        // Determine if the update contains only subtractive operations (REMOVE
        // and/or DELETE). When such an update targets a non-existent item,
        // DynamoDB does NOT create the item.
        let is_subtractive_only = input.update_expression.as_ref().is_some_and(|expr| {
            let upper = expr.trim().to_ascii_uppercase();
            !upper.contains("SET ") && !upper.contains("ADD ")
        });

        // Parse and apply update expression. Keep the parsed AST for
        // computing UPDATED_OLD / UPDATED_NEW return values later.
        let parsed_update = if let Some(ref update_expr) = input.update_expression {
            let parsed = parse_update(update_expr).map_err(expression_error_to_dynamodb)?;
            let ctx = EvalContext {
                item: &item,
                names: &input.expression_attribute_names,
                values: &input.expression_attribute_values,
            };
            item = ctx
                .apply_update(&parsed)
                .map_err(expression_error_to_dynamodb)?;
            Some(parsed)
        } else {
            None
        };

        // If the original item didn't exist and the update only contains
        // subtractive operations (REMOVE/DELETE), the resulting item would
        // only have key attributes. In this case DynamoDB does NOT create
        // the item.
        let item_not_stored = existing.is_none()
            && is_subtractive_only
            && item_has_only_key_attrs(&item, &table.key_schema);

        if item_not_stored {
            // Do not store the item. Return empty attributes for all ReturnValues
            // variants since there is no old item and no new item to return.
            return Ok(UpdateItemOutput {
                attributes: HashMap::new(),
                consumed_capacity: None,
                item_collection_metrics: None,
            });
        }

        // Validate updated item size.
        let size = calculate_item_size(&item);
        if size > MAX_ITEM_SIZE_BYTES {
            return Err(DynamoDBError::validation(format!(
                "Item size has exceeded the maximum allowed size of {MAX_ITEM_SIZE_BYTES} bytes"
            )));
        }

        // Store updated item.
        let old_item = table
            .storage
            .put_item(item.clone())
            .map_err(storage_error_to_dynamodb)?;

        // Emit stream event if stream is enabled for this table.
        if table
            .stream_specification
            .as_ref()
            .is_some_and(|s| s.stream_enabled)
        {
            let event_name = if existing.is_some() || old_item.is_some() {
                crate::stream::ChangeEventName::Modify
            } else {
                crate::stream::ChangeEventName::Insert
            };
            let keys = extract_key_attributes(&item, &table.key_schema_elements);
            self.emitter.emit(crate::stream::ChangeEvent {
                table_name: table.name.clone(),
                event_name,
                keys,
                old_image: old_item.clone().or_else(|| existing.clone()),
                new_image: Some(item.clone()),
                size_bytes: size,
            });
        }

        let old_for_return = old_item.or(existing);

        let attributes = compute_update_return_values(
            input.return_values.as_ref(),
            old_for_return.as_ref(),
            &item,
            &table.key_schema,
            parsed_update.as_ref(),
            &input.expression_attribute_names,
        );

        Ok(UpdateItemOutput {
            attributes,
            consumed_capacity: None,
            item_collection_metrics: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Query & Scan
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `Query`.
    #[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
    pub fn handle_query(&self, mut input: QueryInput) -> Result<QueryOutput, DynamoDBError> {
        let table = self.state.require_table(&input.table_name)?;

        // Validate Select parameter.
        let has_atg = input
            .attributes_to_get
            .as_ref()
            .is_some_and(|v| !v.is_empty());
        validate_select(
            input.select.as_ref(),
            input.projection_expression.is_some(),
            has_atg,
        )?;

        // Validate Limit: must be > 0 if specified.
        if let Some(limit) = input.limit {
            if limit <= 0 {
                return Err(DynamoDBError::validation("Limit must be greater than 0"));
            }
        }

        // Reject both ProjectionExpression and AttributesToGet.
        if input.projection_expression.is_some() && has_atg {
            return Err(DynamoDBError::validation(
                "Cannot have both AttributesToGet and ProjectionExpression",
            ));
        }

        // Validate AttributesToGet: empty list is not allowed.
        if let Some(ref atg) = input.attributes_to_get {
            if atg.is_empty() {
                return Err(DynamoDBError::validation(
                    "One or more parameter values are not valid. The AttributesToGet parameter \
                     must contain at least one element",
                ));
            }
        }

        // Reject mixing non-expression parameter AttributesToGet with expression
        // parameters (FilterExpression, KeyConditionExpression, ProjectionExpression).
        if has_atg {
            let mut expr_params = Vec::new();
            if input.filter_expression.is_some() {
                expr_params.push("FilterExpression");
            }
            if input.key_condition_expression.is_some() {
                expr_params.push("KeyConditionExpression");
            }
            if input.projection_expression.is_some() {
                expr_params.push("ProjectionExpression");
            }
            if !expr_params.is_empty() {
                return Err(DynamoDBError::validation(format!(
                    "Can not use both expression and non-expression parameters in the same \
                     request: Non-expression parameters: {{AttributesToGet}} Expression \
                     parameters: {{{}}}",
                    expr_params.join(", ")
                )));
            }
        }

        // Reject using both KeyConditions and KeyConditionExpression.
        if !input.key_conditions.is_empty() && input.key_condition_expression.is_some() {
            return Err(DynamoDBError::validation(
                "Can not use both expression and non-expression parameters in the same request: \
                 Non-expression parameters: {KeyConditions} Expression parameters: \
                 {KeyConditionExpression}",
            ));
        }

        // Legacy API: convert KeyConditions to KeyConditionExpression.
        if !input.key_conditions.is_empty() && input.key_condition_expression.is_none() {
            let (expr, names, values) = convert_conditions_to_expression(
                &input.key_conditions,
                None, // KeyConditions are always AND-joined
                "#lckc",
                ":lckv",
            );
            input.key_condition_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Legacy API: convert QueryFilter to FilterExpression.
        if !input.query_filter.is_empty() && input.filter_expression.is_none() {
            let (expr, names, values) = convert_conditions_to_expression(
                &input.query_filter,
                input.conditional_operator.as_ref(),
                "#lcqf",
                ":lcqv",
            );
            input.filter_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Legacy API: convert AttributesToGet to ProjectionExpression.
        if let Some(ref atg) = input.attributes_to_get {
            if !atg.is_empty() && input.projection_expression.is_none() {
                input.projection_expression = Some(convert_attributes_to_get(atg));
            }
        }

        // Parse key condition expression (must exist for Query).
        let key_condition = input.key_condition_expression.as_deref().ok_or_else(|| {
            DynamoDBError::validation("KeyConditionExpression is required for Query")
        })?;

        // Reject empty key condition expression (before parsing attempt).
        if key_condition.trim().is_empty() {
            return Err(DynamoDBError::validation(
                "Invalid KeyConditionExpression: The expression can not be empty;",
            ));
        }

        // Reject empty filter expression (before parsing attempt).
        validate_filter_not_empty(input.filter_expression.as_deref())?;

        // Resolve the effective key schema: use GSI key schema if querying an
        // index, otherwise use the table's primary key schema.
        let gsi_key_schema = if let Some(ref index_name) = input.index_name {
            let gsi = table
                .gsi_definitions
                .iter()
                .find(|g| g.index_name == *index_name)
                .ok_or_else(|| {
                    DynamoDBError::validation(format!(
                        "The table does not have the specified index: {index_name}"
                    ))
                })?;
            Some(parse_key_schema(
                &gsi.key_schema,
                &table.attribute_definitions,
            )?)
        } else {
            None
        };
        let effective_key_schema = gsi_key_schema.as_ref().unwrap_or(&table.key_schema);

        // Validate unused expression attribute names/values.
        {
            let mut used_names = HashSet::new();
            let mut used_values = HashSet::new();
            {
                let parsed =
                    parse_condition(key_condition).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&parsed, &mut used_names);
                collect_values_from_expr(&parsed, &mut used_values);
            }
            if let Some(ref filter) = input.filter_expression {
                let parsed = parse_condition(filter).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&parsed, &mut used_names);
                collect_values_from_expr(&parsed, &mut used_values);

                // FilterExpression must not reference key attributes of the
                // index being queried.
                validate_filter_no_key_attrs(
                    &parsed,
                    effective_key_schema,
                    &input.expression_attribute_names,
                )?;
            }
            if let Some(ref proj) = input.projection_expression {
                let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
                collect_names_from_projection(&paths, &mut used_names);
            }
            validate_no_unresolved_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_values(&input.expression_attribute_values, &used_values)?;
        }

        let expr = parse_condition(key_condition).map_err(expression_error_to_dynamodb)?;

        // Validate key condition against the effective key schema (table or GSI).
        validate_key_condition_expr(
            &expr,
            effective_key_schema,
            &input.expression_attribute_names,
        )?;

        // Extract partition key value and sort condition from the parsed expression.
        let (partition_value, sort_condition) = extract_key_condition(
            &expr,
            effective_key_schema,
            &input.expression_attribute_names,
            &input.expression_attribute_values,
        )?;

        let scan_forward = input.scan_index_forward.unwrap_or(true);
        let limit = input
            .limit
            .map(|l| usize::try_from(l.max(0)).unwrap_or(usize::MAX));

        // Branch: table query vs. GSI query.
        let (mut items, last_evaluated_key) = if gsi_key_schema.is_some() {
            query_gsi(
                &table,
                effective_key_schema,
                &partition_value,
                sort_condition.as_ref(),
                scan_forward,
                limit,
                &input.exclusive_start_key,
            )
        } else {
            let exclusive_start_sort = if input.exclusive_start_key.is_empty() {
                None
            } else {
                let start_pk = extract_primary_key(&table.key_schema, &input.exclusive_start_key)
                    .map_err(storage_error_to_dynamodb)?;
                start_pk.sort_key
            };

            let (items, last_key_sort) = table.storage.query(
                &partition_value,
                sort_condition.as_ref(),
                scan_forward,
                limit,
                exclusive_start_sort.as_ref(),
            );

            let last_evaluated_key = last_key_sort.map(|pk| {
                let sort_av = pk
                    .sort_key
                    .as_ref()
                    .and_then(SortableAttributeValue::to_attribute_value);
                build_last_evaluated_key(&table.key_schema, &pk.partition_key, sort_av.as_ref())
            });

            (items, last_evaluated_key)
        };

        let scanned_count = i32::try_from(items.len()).unwrap_or(i32::MAX);

        // Apply filter expression if present.
        if let Some(ref filter) = input.filter_expression {
            let filter_expr = parse_condition(filter).map_err(expression_error_to_dynamodb)?;
            items = items
                .into_iter()
                .map(|item| {
                    let ctx = EvalContext {
                        item: &item,
                        names: &input.expression_attribute_names,
                        values: &input.expression_attribute_values,
                    };
                    let matched = ctx
                        .evaluate(&filter_expr)
                        .map_err(expression_error_to_dynamodb)?;
                    Ok((item, matched))
                })
                .collect::<Result<Vec<_>, DynamoDBError>>()?
                .into_iter()
                .filter_map(|(item, matched)| matched.then_some(item))
                .collect();
        }

        // Apply projection expression if present.
        if let Some(ref proj) = input.projection_expression {
            let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
            items = items
                .into_iter()
                .map(|item| {
                    let ctx = EvalContext {
                        item: &item,
                        names: &input.expression_attribute_names,
                        values: &HashMap::new(),
                    };
                    ctx.apply_projection(&paths)
                })
                .collect();
        }

        let count = i32::try_from(items.len()).unwrap_or(i32::MAX);

        // If Select=COUNT, return only the count (no items).
        if input.select == Some(Select::Count) {
            return Ok(QueryOutput {
                items: None,
                count,
                scanned_count,
                last_evaluated_key: last_evaluated_key.unwrap_or_default(),
                consumed_capacity: None,
            });
        }

        Ok(QueryOutput {
            items: Some(items),
            count,
            scanned_count,
            last_evaluated_key: last_evaluated_key.unwrap_or_default(),
            consumed_capacity: None,
        })
    }

    /// Handle `Scan`.
    #[allow(clippy::needless_pass_by_value, clippy::too_many_lines)]
    pub fn handle_scan(&self, mut input: ScanInput) -> Result<ScanOutput, DynamoDBError> {
        let table = self.state.require_table(&input.table_name)?;

        let has_atg = input
            .attributes_to_get
            .as_ref()
            .is_some_and(|v| !v.is_empty());
        // Validate Select parameter.
        validate_select(
            input.select.as_ref(),
            input.projection_expression.is_some(),
            has_atg,
        )?;

        // Reject both ProjectionExpression and AttributesToGet.
        if input.projection_expression.is_some() && has_atg {
            return Err(DynamoDBError::validation(
                "Cannot have both AttributesToGet and ProjectionExpression",
            ));
        }

        // Validate AttributesToGet: empty list is not allowed.
        if let Some(ref atg) = input.attributes_to_get {
            if atg.is_empty() {
                return Err(DynamoDBError::validation(
                    "One or more parameter values are not valid. The AttributesToGet parameter \
                     must contain at least one element",
                ));
            }
        }

        // Legacy API: convert ScanFilter to FilterExpression.
        if !input.scan_filter.is_empty() && input.filter_expression.is_none() {
            let (expr, names, values) = convert_conditions_to_expression(
                &input.scan_filter,
                input.conditional_operator.as_ref(),
                "#lcsf",
                ":lcsv",
            );
            input.filter_expression = Some(expr);
            merge_expression_names(&mut input.expression_attribute_names, names);
            merge_expression_values(&mut input.expression_attribute_values, values);
        }

        // Legacy API: convert AttributesToGet to ProjectionExpression.
        if let Some(ref atg) = input.attributes_to_get {
            if !atg.is_empty() && input.projection_expression.is_none() {
                input.projection_expression = Some(convert_attributes_to_get(atg));
            }
        }

        // Reject empty filter expression (before parsing attempt).
        validate_filter_not_empty(input.filter_expression.as_deref())?;

        // Validate unused expression attribute names/values.
        {
            let mut used_names = HashSet::new();
            let mut used_values = HashSet::new();
            if let Some(ref filter) = input.filter_expression {
                let parsed = parse_condition(filter).map_err(expression_error_to_dynamodb)?;
                collect_names_from_expr(&parsed, &mut used_names);
                collect_values_from_expr(&parsed, &mut used_values);
            }
            if let Some(ref proj) = input.projection_expression {
                let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
                collect_names_from_projection(&paths, &mut used_names);
            }
            validate_no_unused_names(&input.expression_attribute_names, &used_names)?;
            validate_no_unused_values(&input.expression_attribute_values, &used_values)?;
        }

        let limit = input
            .limit
            .map(|l| usize::try_from(l.max(0)).unwrap_or(usize::MAX));
        let exclusive_start = if input.exclusive_start_key.is_empty() {
            None
        } else {
            Some(
                extract_primary_key(&table.key_schema, &input.exclusive_start_key)
                    .map_err(storage_error_to_dynamodb)?,
            )
        };

        // Validate and extract parallel scan parameters.
        let (segment, total_segments) = validate_parallel_scan(&input, exclusive_start.as_ref())?;

        let (mut items, last_key) =
            table
                .storage
                .scan(limit, exclusive_start.as_ref(), segment, total_segments);

        let scanned_count = i32::try_from(items.len()).unwrap_or(i32::MAX);

        // Apply filter expression if present.
        if let Some(ref filter) = input.filter_expression {
            let filter_expr = parse_condition(filter).map_err(expression_error_to_dynamodb)?;
            items = items
                .into_iter()
                .map(|item| {
                    let ctx = EvalContext {
                        item: &item,
                        names: &input.expression_attribute_names,
                        values: &input.expression_attribute_values,
                    };
                    let matched = ctx
                        .evaluate(&filter_expr)
                        .map_err(expression_error_to_dynamodb)?;
                    Ok((item, matched))
                })
                .collect::<Result<Vec<_>, DynamoDBError>>()?
                .into_iter()
                .filter_map(|(item, matched)| matched.then_some(item))
                .collect();
        }

        // Apply projection expression if present.
        if let Some(ref proj) = input.projection_expression {
            let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
            items = items
                .into_iter()
                .map(|item| {
                    let ctx = EvalContext {
                        item: &item,
                        names: &input.expression_attribute_names,
                        values: &HashMap::new(),
                    };
                    ctx.apply_projection(&paths)
                })
                .collect();
        }

        let count = i32::try_from(items.len()).unwrap_or(i32::MAX);

        let last_evaluated_key = last_key.map(|pk| {
            let sort_av = pk
                .sort_key
                .as_ref()
                .and_then(SortableAttributeValue::to_attribute_value);
            build_last_evaluated_key(&table.key_schema, &pk.partition_key, sort_av.as_ref())
        });

        // If Select=COUNT, return only the count (no items).
        if input.select == Some(Select::Count) {
            return Ok(ScanOutput {
                items: None,
                count,
                scanned_count,
                last_evaluated_key: last_evaluated_key.unwrap_or_default(),
                consumed_capacity: None,
            });
        }

        Ok(ScanOutput {
            items: Some(items),
            count,
            scanned_count,
            last_evaluated_key: last_evaluated_key.unwrap_or_default(),
            consumed_capacity: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Batch operations
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `BatchGetItem`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_batch_get_item(
        &self,
        input: BatchGetItemInput,
    ) -> Result<BatchGetItemOutput, DynamoDBError> {
        // Enforce 100-item limit across all tables.
        let total_keys: usize = input.request_items.values().map(|ka| ka.keys.len()).sum();
        if total_keys > 100 {
            return Err(DynamoDBError::validation(
                "Too many items requested for the BatchGetItem call",
            ));
        }

        let mut responses: HashMap<String, Vec<HashMap<String, AttributeValue>>> = HashMap::new();

        for (table_name, keys_and_attrs) in &input.request_items {
            let table = self.state.require_table(table_name)?;

            // Detect duplicate keys within this table.
            detect_duplicate_keys(&table.key_schema, keys_and_attrs.keys.iter())?;

            // Determine effective projection: ProjectionExpression takes priority,
            // then AttributesToGet.
            let effective_projection = if keys_and_attrs.projection_expression.is_some() {
                keys_and_attrs.projection_expression.clone()
            } else if !keys_and_attrs.attributes_to_get.is_empty() {
                Some(convert_attributes_to_get(&keys_and_attrs.attributes_to_get))
            } else {
                None
            };

            // Validate unused expression attribute names if projection is set.
            if let Some(ref proj) = effective_projection {
                if let Some(ref ean) = keys_and_attrs.expression_attribute_names {
                    if !ean.is_empty() {
                        let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
                        let mut used_names = HashSet::new();
                        collect_names_from_projection(&paths, &mut used_names);
                        validate_no_unused_names(ean, &used_names)?;
                    }
                }
            }

            let mut table_items = Vec::new();

            for key in &keys_and_attrs.keys {
                let pk = extract_primary_key(&table.key_schema, key)
                    .map_err(storage_error_to_dynamodb)?;
                if let Some(item) = table.storage.get_item(&pk) {
                    // Apply projection if specified.
                    let item = if let Some(ref proj) = effective_projection {
                        let paths = parse_projection(proj).map_err(projection_error_to_dynamodb)?;
                        let names = keys_and_attrs
                            .expression_attribute_names
                            .clone()
                            .unwrap_or_default();
                        let ctx = EvalContext {
                            item: &item,
                            names: &names,
                            values: &HashMap::new(),
                        };
                        ctx.apply_projection(&paths)
                    } else {
                        item
                    };
                    table_items.push(item);
                }
            }

            // Always include the table in responses (even if empty).
            responses.insert(table_name.clone(), table_items);
        }

        Ok(BatchGetItemOutput {
            responses,
            unprocessed_keys: HashMap::new(),
            consumed_capacity: Vec::new(),
        })
    }

    /// Handle `BatchWriteItem`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_batch_write_item(
        &self,
        input: BatchWriteItemInput,
    ) -> Result<BatchWriteItemOutput, DynamoDBError> {
        // Enforce 25-item limit across all tables.
        let total_writes: usize = input.request_items.values().map(Vec::len).sum();
        if total_writes > 25 {
            return Err(DynamoDBError::validation(format!(
                "Too many items in the BatchWriteItem request; the request length {total_writes} \
                 exceeds the limit of 25"
            )));
        }

        // Validation pass: validate all items before writing any (atomic failure).
        for (table_name, write_requests) in &input.request_items {
            let table = self.state.require_table(table_name)?;

            // Detect duplicate keys within this table's write requests.
            let key_items: Vec<&HashMap<String, AttributeValue>> = write_requests
                .iter()
                .filter_map(|wr| {
                    wr.put_request
                        .as_ref()
                        .map(|p| &p.item)
                        .or(wr.delete_request.as_ref().map(|d| &d.key))
                })
                .collect();
            detect_duplicate_keys(&table.key_schema, key_items.into_iter())?;

            for wr in write_requests {
                if let Some(ref put) = wr.put_request {
                    validate_key_not_empty(&table.key_schema, &put.item)?;
                    let size = calculate_item_size(&put.item);
                    if size > MAX_ITEM_SIZE_BYTES {
                        return Err(DynamoDBError::validation(format!(
                            "Item size has exceeded the maximum allowed size of \
                             {MAX_ITEM_SIZE_BYTES} bytes"
                        )));
                    }
                    // Validate key schema compliance (catches wrong key types).
                    extract_primary_key(&table.key_schema, &put.item)
                        .map_err(storage_error_to_dynamodb)?;
                } else if let Some(ref del) = wr.delete_request {
                    extract_primary_key(&table.key_schema, &del.key)
                        .map_err(storage_error_to_dynamodb)?;
                }
            }
        }

        // Execution pass: all validations passed, now execute writes.
        for (table_name, write_requests) in &input.request_items {
            let table = self.state.require_table(table_name)?;
            let stream_enabled = table
                .stream_specification
                .as_ref()
                .is_some_and(|s| s.stream_enabled);

            for wr in write_requests {
                if let Some(ref put) = wr.put_request {
                    let old = table
                        .storage
                        .put_item(put.item.clone())
                        .map_err(storage_error_to_dynamodb)?;

                    if stream_enabled {
                        let event_name = if old.is_some() {
                            crate::stream::ChangeEventName::Modify
                        } else {
                            crate::stream::ChangeEventName::Insert
                        };
                        let keys = extract_key_attributes(&put.item, &table.key_schema_elements);
                        let size = calculate_item_size(&put.item);
                        self.emitter.emit(crate::stream::ChangeEvent {
                            table_name: table.name.clone(),
                            event_name,
                            keys,
                            old_image: old,
                            new_image: Some(put.item.clone()),
                            size_bytes: size,
                        });
                    }
                } else if let Some(ref del) = wr.delete_request {
                    let pk = extract_primary_key(&table.key_schema, &del.key)
                        .map_err(storage_error_to_dynamodb)?;
                    let old = table.storage.delete_item(&pk);

                    if stream_enabled {
                        if let Some(ref old_item) = old {
                            let keys = extract_key_attributes(old_item, &table.key_schema_elements);
                            let size = calculate_item_size(old_item);
                            self.emitter.emit(crate::stream::ChangeEvent {
                                table_name: table.name.clone(),
                                event_name: crate::stream::ChangeEventName::Remove,
                                keys,
                                old_image: Some(old_item.clone()),
                                new_image: None,
                                size_bytes: size,
                            });
                        }
                    }
                }
            }
        }

        Ok(BatchWriteItemOutput {
            unprocessed_items: HashMap::new(),
            item_collection_metrics: HashMap::new(),
            consumed_capacity: Vec::new(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tagging operations
// ---------------------------------------------------------------------------

/// Maximum number of tags per resource.
const MAX_TAGS_PER_RESOURCE: usize = 50;

/// Maximum tag key length (characters).
const MAX_TAG_KEY_LENGTH: usize = 128;

/// Maximum tag value length (characters).
const MAX_TAG_VALUE_LENGTH: usize = 256;

impl RustStackDynamoDB {
    /// Resolve a DynamoDB resource ARN to a table name.
    fn resolve_table_from_arn(arn: &str) -> Result<&str, DynamoDBError> {
        arn.rsplit_once('/')
            .map(|(_, name)| name)
            .filter(|name| !name.is_empty())
            .ok_or_else(|| DynamoDBError::validation("Invalid resource ARN"))
    }

    /// Handle `TagResource`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_tag_resource(
        &self,
        input: TagResourceInput,
    ) -> Result<TagResourceOutput, DynamoDBError> {
        let table_name = Self::resolve_table_from_arn(&input.resource_arn)?;
        let table = self.state.require_table(table_name)?;

        // Validate each tag.
        for tag in &input.tags {
            if tag.key.is_empty() || tag.key.len() > MAX_TAG_KEY_LENGTH {
                return Err(DynamoDBError::validation(format!(
                    "Tag key must be between 1 and {MAX_TAG_KEY_LENGTH} characters long"
                )));
            }
            if tag.value.len() > MAX_TAG_VALUE_LENGTH {
                return Err(DynamoDBError::validation(format!(
                    "Tag value must be no more than {MAX_TAG_VALUE_LENGTH} characters long"
                )));
            }
            if tag.key.starts_with("aws:") {
                return Err(DynamoDBError::validation(
                    "Tag keys starting with 'aws:' are reserved for system use",
                ));
            }
        }

        let mut tags = table.tags.write();

        // Merge: update existing keys, add new ones.
        for new_tag in &input.tags {
            if let Some(existing) = tags.iter_mut().find(|t| t.key == new_tag.key) {
                existing.value.clone_from(&new_tag.value);
            } else {
                tags.push(new_tag.clone());
            }
        }

        // Validate total count.
        if tags.len() > MAX_TAGS_PER_RESOURCE {
            return Err(DynamoDBError::validation(format!(
                "The number of tags exceeds the limit of {MAX_TAGS_PER_RESOURCE}"
            )));
        }

        Ok(TagResourceOutput {})
    }

    /// Handle `UntagResource`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_untag_resource(
        &self,
        input: UntagResourceInput,
    ) -> Result<UntagResourceOutput, DynamoDBError> {
        let table_name = Self::resolve_table_from_arn(&input.resource_arn)?;
        let table = self.state.require_table(table_name)?;

        let keys_to_remove: HashSet<&str> = input.tag_keys.iter().map(String::as_str).collect();
        let mut tags = table.tags.write();
        tags.retain(|t| !keys_to_remove.contains(t.key.as_str()));

        Ok(UntagResourceOutput {})
    }

    /// Handle `ListTagsOfResource`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_list_tags_of_resource(
        &self,
        input: ListTagsOfResourceInput,
    ) -> Result<ListTagsOfResourceOutput, DynamoDBError> {
        let table_name = Self::resolve_table_from_arn(&input.resource_arn)?;
        let table = self.state.require_table(table_name)?;

        let tags = table.tags.read().clone();
        Ok(ListTagsOfResourceOutput {
            tags: Some(tags),
            next_token: None,
        })
    }
}

// ---------------------------------------------------------------------------
// Time to Live operations
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `UpdateTimeToLive`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_update_time_to_live(
        &self,
        input: UpdateTimeToLiveInput,
    ) -> Result<UpdateTimeToLiveOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        if input.time_to_live_specification.attribute_name.is_empty() {
            return Err(DynamoDBError::validation(
                "TimeToLiveSpecification AttributeName must not be empty",
            ));
        }

        let spec = input.time_to_live_specification;
        *table.ttl.write() = Some(spec.clone());

        Ok(UpdateTimeToLiveOutput {
            time_to_live_specification: Some(spec),
        })
    }

    /// Handle `DescribeTimeToLive`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_describe_time_to_live(
        &self,
        input: DescribeTimeToLiveInput,
    ) -> Result<DescribeTimeToLiveOutput, DynamoDBError> {
        validate_table_name(&input.table_name)?;
        let table = self.state.require_table(&input.table_name)?;

        let ttl_guard = table.ttl.read();
        let description = match ttl_guard.as_ref() {
            Some(spec) => TimeToLiveDescription {
                attribute_name: Some(spec.attribute_name.clone()),
                time_to_live_status: Some(if spec.enabled {
                    "ENABLED".to_owned()
                } else {
                    "DISABLED".to_owned()
                }),
            },
            None => TimeToLiveDescription {
                attribute_name: None,
                time_to_live_status: Some("DISABLED".to_owned()),
            },
        };

        Ok(DescribeTimeToLiveOutput {
            time_to_live_description: Some(description),
        })
    }
}

// ---------------------------------------------------------------------------
// Describe operations
// ---------------------------------------------------------------------------

impl RustStackDynamoDB {
    /// Handle `DescribeLimits`.
    ///
    /// Returns hardcoded account and table capacity limits matching the default
    /// AWS DynamoDB provisioned-mode limits.
    pub fn handle_describe_limits(
        &self,
        _input: DescribeLimitsInput,
    ) -> Result<DescribeLimitsOutput, DynamoDBError> {
        Ok(DescribeLimitsOutput {
            account_max_read_capacity_units: Some(80_000),
            account_max_write_capacity_units: Some(80_000),
            table_max_read_capacity_units: Some(40_000),
            table_max_write_capacity_units: Some(40_000),
        })
    }

    /// Handle `DescribeEndpoints`.
    ///
    /// Returns a single endpoint for the configured region with a 1440-minute
    /// (24 hour) cache period, matching the real DynamoDB behaviour.
    pub fn handle_describe_endpoints(
        &self,
        _input: DescribeEndpointsInput,
    ) -> Result<DescribeEndpointsOutput, DynamoDBError> {
        let address = format!("dynamodb.{}.amazonaws.com", self.config.default_region);
        Ok(DescribeEndpointsOutput {
            endpoints: vec![Endpoint {
                address,
                cache_period_in_minutes: 1440,
            }],
        })
    }
}

// ---------------------------------------------------------------------------
// Transaction operations
// ---------------------------------------------------------------------------

/// Maximum number of items in a transaction.
const MAX_TRANSACT_ITEMS: usize = 100;

impl RustStackDynamoDB {
    /// Handle `TransactGetItems`.
    #[allow(clippy::needless_pass_by_value)]
    pub fn handle_transact_get_items(
        &self,
        input: TransactGetItemsInput,
    ) -> Result<TransactGetItemsOutput, DynamoDBError> {
        if input.transact_items.is_empty() {
            return Err(DynamoDBError::validation(
                "1 validation error detected: Value null at 'transactItems' failed to satisfy \
                 constraint: Member must not be null",
            ));
        }
        if input.transact_items.len() > MAX_TRANSACT_ITEMS {
            return Err(DynamoDBError::validation(format!(
                "1 validation error detected: Value '[TransactGetItem]' at 'transactItems' failed \
                 to satisfy constraint: Member must have length less than or equal to \
                 {MAX_TRANSACT_ITEMS}"
            )));
        }

        let mut responses = Vec::with_capacity(input.transact_items.len());

        for transact_item in &input.transact_items {
            let get = &transact_item.get;
            let table = self.state.require_table(&get.table_name)?;
            let pk = extract_primary_key(&table.key_schema, &get.key)
                .map_err(storage_error_to_dynamodb)?;

            let item = table.storage.get_item(&pk);

            // Apply projection if specified.
            let result_item = match (item, &get.projection_expression) {
                (Some(found_item), Some(projection)) => {
                    let names = get.expression_attribute_names.as_ref();
                    let empty_names = HashMap::new();
                    let names_ref = names.unwrap_or(&empty_names);
                    let paths =
                        parse_projection(projection).map_err(projection_error_to_dynamodb)?;
                    let empty_values = HashMap::new();
                    let ctx = EvalContext {
                        item: &found_item,
                        names: names_ref,
                        values: &empty_values,
                    };
                    let projected = ctx.apply_projection(&paths);
                    if projected.is_empty() {
                        None
                    } else {
                        Some(projected)
                    }
                }
                (Some(found_item), None) => Some(found_item),
                (None, _) => None,
            };

            responses.push(ItemResponse { item: result_item });
        }

        Ok(TransactGetItemsOutput {
            consumed_capacity: Vec::new(),
            responses: Some(responses),
        })
    }

    /// Handle `TransactWriteItems`.
    #[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
    pub fn handle_transact_write_items(
        &self,
        input: TransactWriteItemsInput,
    ) -> Result<TransactWriteItemsOutput, DynamoDBError> {
        if input.transact_items.is_empty() {
            return Err(DynamoDBError::validation(
                "1 validation error detected: Value null at 'transactItems' failed to satisfy \
                 constraint: Member must not be null",
            ));
        }
        if input.transact_items.len() > MAX_TRANSACT_ITEMS {
            return Err(DynamoDBError::validation(format!(
                "1 validation error detected: Value '[TransactWriteItem]' at 'transactItems' \
                 failed to satisfy constraint: Member must have length less than or equal to \
                 {MAX_TRANSACT_ITEMS}"
            )));
        }

        // Phase 1: Validate each item has exactly one action and collect
        // (table_name, primary_key) pairs for duplicate detection.
        let mut seen_keys: HashSet<(String, PrimaryKey)> = HashSet::new();

        for (idx, item) in input.transact_items.iter().enumerate() {
            let action_count = [
                item.condition_check.is_some(),
                item.put.is_some(),
                item.delete.is_some(),
                item.update.is_some(),
            ]
            .iter()
            .filter(|&&b| b)
            .count();

            if action_count != 1 {
                return Err(DynamoDBError::validation(format!(
                    "TransactItems[{idx}] must specify exactly one of ConditionCheck, Put, \
                     Delete, or Update"
                )));
            }

            // Extract (table_name, key) for duplicate detection.
            let (table_name, key_map) = if let Some(ref cc) = item.condition_check {
                (cc.table_name.as_str(), &cc.key)
            } else if let Some(ref put) = item.put {
                let table = self.state.require_table(&put.table_name)?;
                let pk = extract_primary_key(&table.key_schema, &put.item)
                    .map_err(storage_error_to_dynamodb)?;
                if !seen_keys.insert((put.table_name.clone(), pk)) {
                    return Err(DynamoDBError::validation(
                        "Transaction request cannot include multiple operations on one item",
                    ));
                }
                continue;
            } else if let Some(ref del) = item.delete {
                (del.table_name.as_str(), &del.key)
            } else if let Some(ref upd) = item.update {
                (upd.table_name.as_str(), &upd.key)
            } else {
                continue;
            };

            let table = self.state.require_table(table_name)?;
            let pk = extract_primary_key(&table.key_schema, key_map)
                .map_err(storage_error_to_dynamodb)?;
            if !seen_keys.insert((table_name.to_owned(), pk)) {
                return Err(DynamoDBError::validation(
                    "Transaction request cannot include multiple operations on one item",
                ));
            }
        }

        // Phase 2: Evaluate all condition expressions.
        let mut cancellation_reasons: Vec<CancellationReason> = vec![
            CancellationReason {
                code: Some("None".to_owned()),
                message: Some("None".to_owned()),
                item: None,
            };
            input.transact_items.len()
        ];
        let mut any_cancelled = false;

        for (idx, item) in input.transact_items.iter().enumerate() {
            let condition_result = self.evaluate_transact_write_condition(item);
            match condition_result {
                Ok(()) => {}
                Err(reason) => {
                    cancellation_reasons[idx] = reason;
                    any_cancelled = true;
                }
            }
        }

        if any_cancelled {
            return Err(DynamoDBError::transaction_cancelled(cancellation_reasons));
        }

        // Phase 3: Apply all writes.
        for item in &input.transact_items {
            if let Some(ref put) = item.put {
                let table = self.state.require_table(&put.table_name)?;
                let old = table
                    .storage
                    .put_item(put.item.clone())
                    .map_err(storage_error_to_dynamodb)?;

                if table
                    .stream_specification
                    .as_ref()
                    .is_some_and(|s| s.stream_enabled)
                {
                    let event_name = if old.is_some() {
                        crate::stream::ChangeEventName::Modify
                    } else {
                        crate::stream::ChangeEventName::Insert
                    };
                    let keys = extract_key_attributes(&put.item, &table.key_schema_elements);
                    let size = calculate_item_size(&put.item);
                    self.emitter.emit(crate::stream::ChangeEvent {
                        table_name: table.name.clone(),
                        event_name,
                        keys,
                        old_image: old,
                        new_image: Some(put.item.clone()),
                        size_bytes: size,
                    });
                }
            } else if let Some(ref del) = item.delete {
                let table = self.state.require_table(&del.table_name)?;
                let pk = extract_primary_key(&table.key_schema, &del.key)
                    .map_err(storage_error_to_dynamodb)?;
                let old = table.storage.delete_item(&pk);

                if table
                    .stream_specification
                    .as_ref()
                    .is_some_and(|s| s.stream_enabled)
                {
                    if let Some(ref old_item) = old {
                        let keys = extract_key_attributes(old_item, &table.key_schema_elements);
                        let size = calculate_item_size(old_item);
                        self.emitter.emit(crate::stream::ChangeEvent {
                            table_name: table.name.clone(),
                            event_name: crate::stream::ChangeEventName::Remove,
                            keys,
                            old_image: Some(old_item.clone()),
                            new_image: None,
                            size_bytes: size,
                        });
                    }
                }
            } else if let Some(ref upd) = item.update {
                let table = self.state.require_table(&upd.table_name)?;
                let pk = extract_primary_key(&table.key_schema, &upd.key)
                    .map_err(storage_error_to_dynamodb)?;
                let existing = table.storage.get_item(&pk);
                let current = existing.clone().unwrap_or_else(|| upd.key.clone());

                let names = upd.expression_attribute_names.as_ref();
                let values = upd.expression_attribute_values.as_ref();
                let empty_names = HashMap::new();
                let empty_values = HashMap::new();
                let names_ref = names.unwrap_or(&empty_names);
                let values_ref = values.unwrap_or(&empty_values);

                let parsed =
                    parse_update(&upd.update_expression).map_err(expression_error_to_dynamodb)?;
                let ctx = EvalContext {
                    item: &current,
                    names: names_ref,
                    values: values_ref,
                };
                let updated = ctx
                    .apply_update(&parsed)
                    .map_err(expression_error_to_dynamodb)?;

                let old = table
                    .storage
                    .put_item(updated.clone())
                    .map_err(storage_error_to_dynamodb)?;

                if table
                    .stream_specification
                    .as_ref()
                    .is_some_and(|s| s.stream_enabled)
                {
                    let event_name = if existing.is_some() {
                        crate::stream::ChangeEventName::Modify
                    } else {
                        crate::stream::ChangeEventName::Insert
                    };
                    let keys = extract_key_attributes(&updated, &table.key_schema_elements);
                    let size = calculate_item_size(&updated);
                    self.emitter.emit(crate::stream::ChangeEvent {
                        table_name: table.name.clone(),
                        event_name,
                        keys,
                        old_image: old,
                        new_image: Some(updated),
                        size_bytes: size,
                    });
                }
            }
            // ConditionCheck: no mutation needed.
        }

        Ok(TransactWriteItemsOutput {
            consumed_capacity: Vec::new(),
            item_collection_metrics: HashMap::new(),
        })
    }

    /// Evaluate a condition expression for a single transaction write item.
    ///
    /// Returns `Ok(())` if the condition passes (or no condition exists),
    /// or a `CancellationReason` if the condition fails.
    fn evaluate_transact_write_condition(
        &self,
        item: &ruststack_dynamodb_model::types::TransactWriteItem,
    ) -> Result<(), CancellationReason> {
        if let Some(ref cc) = item.condition_check {
            self.evaluate_condition_for_key(
                &cc.table_name,
                &cc.key,
                Some(&cc.condition_expression),
                cc.expression_attribute_names.as_ref(),
                cc.expression_attribute_values.as_ref(),
                cc.return_values_on_condition_check_failure.as_deref(),
            )
        } else if let Some(ref put) = item.put {
            if let Some(ref condition) = put.condition_expression {
                self.evaluate_transact_put_condition(put, condition)
            } else {
                Ok(())
            }
        } else if let Some(ref del) = item.delete {
            self.evaluate_condition_for_key(
                &del.table_name,
                &del.key,
                del.condition_expression.as_deref(),
                del.expression_attribute_names.as_ref(),
                del.expression_attribute_values.as_ref(),
                del.return_values_on_condition_check_failure.as_deref(),
            )
        } else if let Some(ref upd) = item.update {
            self.evaluate_condition_for_key(
                &upd.table_name,
                &upd.key,
                upd.condition_expression.as_deref(),
                upd.expression_attribute_names.as_ref(),
                upd.expression_attribute_values.as_ref(),
                upd.return_values_on_condition_check_failure.as_deref(),
            )
        } else {
            Ok(())
        }
    }

    /// Evaluate a condition expression for a `TransactPut` action.
    ///
    /// Put actions derive their primary key from the item rather than a
    /// separate `Key` field, requiring special handling.
    fn evaluate_transact_put_condition(
        &self,
        put: &ruststack_dynamodb_model::types::TransactPut,
        condition: &str,
    ) -> Result<(), CancellationReason> {
        let table = self
            .state
            .require_table(&put.table_name)
            .map_err(|e| CancellationReason {
                code: Some("ValidationException".to_owned()),
                message: Some(e.message.clone()),
                item: None,
            })?;
        let pk =
            extract_primary_key(&table.key_schema, &put.item).map_err(|e| CancellationReason {
                code: Some("ValidationException".to_owned()),
                message: Some(e.to_string()),
                item: None,
            })?;
        let key_map: HashMap<String, AttributeValue> =
            extract_key_attributes(&put.item, &table.key_schema_elements);
        let existing = table.storage.get_item(&pk);
        let empty = HashMap::new();
        let item_ref = existing.as_ref().unwrap_or(&empty);
        let empty_names = HashMap::new();
        let empty_values = HashMap::new();
        let names = put
            .expression_attribute_names
            .as_ref()
            .unwrap_or(&empty_names);
        let values = put
            .expression_attribute_values
            .as_ref()
            .unwrap_or(&empty_values);

        let expr = parse_condition(condition).map_err(|e| CancellationReason {
            code: Some("ValidationException".to_owned()),
            message: Some(e.to_string()),
            item: None,
        })?;
        let ctx = EvalContext {
            item: item_ref,
            names,
            values,
        };
        let result = ctx.evaluate(&expr).map_err(|e| CancellationReason {
            code: Some("ValidationException".to_owned()),
            message: Some(e.to_string()),
            item: None,
        })?;
        if !result {
            let return_item =
                if put.return_values_on_condition_check_failure.as_deref() == Some("ALL_OLD") {
                    existing
                } else {
                    None
                };
            return Err(CancellationReason {
                code: Some("ConditionalCheckFailed".to_owned()),
                message: Some("The conditional request failed".to_owned()),
                item: return_item.or(Some(key_map)),
            });
        }
        Ok(())
    }

    /// Evaluate a condition expression for a given table and key.
    fn evaluate_condition_for_key(
        &self,
        table_name: &str,
        key: &HashMap<String, AttributeValue>,
        condition_expression: Option<&str>,
        expression_names: Option<&HashMap<String, String>>,
        expression_values: Option<&HashMap<String, AttributeValue>>,
        return_values_on_failure: Option<&str>,
    ) -> Result<(), CancellationReason> {
        let Some(condition_str) = condition_expression else {
            return Ok(());
        };

        let table = self
            .state
            .require_table(table_name)
            .map_err(|e| CancellationReason {
                code: Some("ValidationException".to_owned()),
                message: Some(e.message.clone()),
                item: None,
            })?;
        let pk = extract_primary_key(&table.key_schema, key).map_err(|e| CancellationReason {
            code: Some("ValidationException".to_owned()),
            message: Some(e.to_string()),
            item: None,
        })?;
        let existing = table.storage.get_item(&pk);
        let empty = HashMap::new();
        let item_ref = existing.as_ref().unwrap_or(&empty);
        let empty_names = HashMap::new();
        let empty_values = HashMap::new();
        let names = expression_names.unwrap_or(&empty_names);
        let values = expression_values.unwrap_or(&empty_values);

        let expr = parse_condition(condition_str).map_err(|e| CancellationReason {
            code: Some("ValidationException".to_owned()),
            message: Some(e.to_string()),
            item: None,
        })?;
        let ctx = EvalContext {
            item: item_ref,
            names,
            values,
        };
        let result = ctx.evaluate(&expr).map_err(|e| CancellationReason {
            code: Some("ValidationException".to_owned()),
            message: Some(e.to_string()),
            item: None,
        })?;

        if !result {
            let return_item = if return_values_on_failure == Some("ALL_OLD") {
                existing
            } else {
                None
            };
            return Err(CancellationReason {
                code: Some("ConditionalCheckFailed".to_owned()),
                message: Some("The conditional request failed".to_owned()),
                item: return_item,
            });
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Legacy API conversion functions
// ---------------------------------------------------------------------------

/// Convert a legacy `Condition` map (used by `KeyConditions`, `QueryFilter`,
/// `ScanFilter`) into an expression string with synthetic attribute name and
/// value placeholders.
///
/// Returns `(expression_string, name_map, value_map)`.
fn convert_conditions_to_expression(
    conditions: &HashMap<String, Condition>,
    conditional_operator: Option<&ConditionalOperator>,
    name_prefix: &str,
    val_prefix: &str,
) -> (
    String,
    HashMap<String, String>,
    HashMap<String, AttributeValue>,
) {
    let joiner = match conditional_operator {
        Some(ConditionalOperator::Or) => " OR ",
        _ => " AND ",
    };

    let mut names: HashMap<String, String> = HashMap::new();
    let mut values: HashMap<String, AttributeValue> = HashMap::new();
    let mut parts: Vec<String> = Vec::new();
    let mut counter: usize = 0;

    // Sort keys for deterministic output (important for tests).
    let mut sorted_keys: Vec<&String> = conditions.keys().collect();
    sorted_keys.sort();

    for attr_name in sorted_keys {
        let Some(condition) = conditions.get(attr_name) else {
            continue;
        };
        let name_placeholder = format!("{name_prefix}{counter}");
        names.insert(name_placeholder.clone(), attr_name.clone());

        let fragment = build_condition_fragment(
            &name_placeholder,
            &condition.comparison_operator,
            &condition.attribute_value_list,
            &mut values,
            &mut counter,
            val_prefix,
        );
        parts.push(fragment);
        counter += 1;
    }

    (parts.join(joiner), names, values)
}

/// Convert a legacy `Expected` map into a `ConditionExpression` string.
///
/// Returns `(expression_string, name_map, value_map)`.
fn convert_expected_to_condition(
    expected: &HashMap<String, ExpectedAttributeValue>,
    conditional_operator: Option<&ConditionalOperator>,
) -> (
    String,
    HashMap<String, String>,
    HashMap<String, AttributeValue>,
) {
    let joiner = match conditional_operator {
        Some(ConditionalOperator::Or) => " OR ",
        _ => " AND ",
    };

    let mut names: HashMap<String, String> = HashMap::new();
    let mut values: HashMap<String, AttributeValue> = HashMap::new();
    let mut parts: Vec<String> = Vec::new();
    let mut counter: usize = 0;

    // Sort keys for deterministic output.
    let mut sorted_keys: Vec<&String> = expected.keys().collect();
    sorted_keys.sort();

    for attr_name in sorted_keys {
        let Some(exp) = expected.get(attr_name) else {
            continue;
        };
        // Use distinct prefixes (#lcexp / :lcexpv) to avoid collisions with
        // placeholders generated by convert_attribute_updates_to_expression
        // (#lcattr / :lcval) when both Expected and AttributeUpdates are
        // present in the same request.
        let name_placeholder = format!("#lcexp{counter}");
        names.insert(name_placeholder.clone(), attr_name.clone());

        if let Some(ref comp_op) = exp.comparison_operator {
            // Extended form: uses comparison_operator with attribute_value_list.
            let fragment = build_expected_condition_fragment(
                &name_placeholder,
                comp_op,
                &exp.attribute_value_list,
                &mut values,
                &mut counter,
            );
            parts.push(fragment);
        } else if let Some(ref value) = exp.value {
            // Simple form with value: attribute must equal value.
            let val_placeholder = format!(":lcexpv{counter}");
            values.insert(val_placeholder.clone(), value.clone());
            parts.push(format!("{name_placeholder} = {val_placeholder}"));
        } else if let Some(false) = exp.exists {
            // exists=false means attribute must not exist.
            parts.push(format!("attribute_not_exists({name_placeholder})"));
        } else {
            // exists=true (or default): attribute must exist.
            parts.push(format!("attribute_exists({name_placeholder})"));
        }

        counter += 1;
    }

    if parts.is_empty() {
        // Fallback: should not happen with non-empty expected map, but be safe.
        return (String::new(), names, values);
    }

    (parts.join(joiner), names, values)
}

/// Convert a legacy `AttributeUpdates` map into an `UpdateExpression` string.
///
/// Returns `(expression_string, name_map, value_map)`.
fn convert_attribute_updates_to_expression(
    updates: &HashMap<String, AttributeValueUpdate>,
) -> (
    String,
    HashMap<String, String>,
    HashMap<String, AttributeValue>,
) {
    let mut names: HashMap<String, String> = HashMap::new();
    let mut values: HashMap<String, AttributeValue> = HashMap::new();

    let mut set_parts: Vec<String> = Vec::new();
    let mut remove_parts: Vec<String> = Vec::new();
    let mut add_parts: Vec<String> = Vec::new();
    let mut delete_parts: Vec<String> = Vec::new();

    let mut counter: usize = 0;

    // Sort keys for deterministic output.
    let mut sorted_keys: Vec<&String> = updates.keys().collect();
    sorted_keys.sort();

    for attr_name in sorted_keys {
        let Some(update) = updates.get(attr_name) else {
            continue;
        };
        let name_placeholder = format!("#lcattr{counter}");
        names.insert(name_placeholder.clone(), attr_name.clone());

        let action = update.action.clone().unwrap_or(AttributeAction::Put);

        match action {
            AttributeAction::Put => {
                if let Some(ref val) = update.value {
                    let val_placeholder = format!(":lcval{counter}");
                    values.insert(val_placeholder.clone(), val.clone());
                    set_parts.push(format!("{name_placeholder} = {val_placeholder}"));
                }
                // PUT without value is a no-op per DynamoDB behaviour.
            }
            AttributeAction::Add => {
                if let Some(ref val) = update.value {
                    let val_placeholder = format!(":lcval{counter}");
                    values.insert(val_placeholder.clone(), val.clone());
                    add_parts.push(format!("{name_placeholder} {val_placeholder}"));
                }
            }
            AttributeAction::Delete => {
                if let Some(ref val) = update.value {
                    // DELETE with value: remove elements from a set.
                    let val_placeholder = format!(":lcval{counter}");
                    values.insert(val_placeholder.clone(), val.clone());
                    delete_parts.push(format!("{name_placeholder} {val_placeholder}"));
                } else {
                    // DELETE without value: remove the attribute entirely.
                    remove_parts.push(name_placeholder.clone());
                }
            }
        }

        counter += 1;
    }

    let mut clauses: Vec<String> = Vec::new();
    if !set_parts.is_empty() {
        clauses.push(format!("SET {}", set_parts.join(", ")));
    }
    if !remove_parts.is_empty() {
        clauses.push(format!("REMOVE {}", remove_parts.join(", ")));
    }
    if !add_parts.is_empty() {
        clauses.push(format!("ADD {}", add_parts.join(", ")));
    }
    if !delete_parts.is_empty() {
        clauses.push(format!("DELETE {}", delete_parts.join(", ")));
    }

    (clauses.join(" "), names, values)
}

/// Convert a legacy `AttributesToGet` list into a `ProjectionExpression` string.
///
/// Since attribute names may contain reserved words or special characters,
/// we emit them directly (without placeholders) joined by commas. This is
/// sufficient because `AttributesToGet` was always simple top-level attribute
/// names.
fn convert_attributes_to_get(attrs: &[String]) -> String {
    attrs.join(", ")
}

/// Build a single condition fragment for a given comparison operator and values.
fn build_condition_fragment(
    name_placeholder: &str,
    op: &ComparisonOperator,
    value_list: &[AttributeValue],
    values: &mut HashMap<String, AttributeValue>,
    counter: &mut usize,
    val_prefix: &str,
) -> String {
    match op {
        ComparisonOperator::Eq => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} = {val_ph}")
        }
        ComparisonOperator::Ne => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} <> {val_ph}")
        }
        ComparisonOperator::Lt => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} < {val_ph}")
        }
        ComparisonOperator::Le => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} <= {val_ph}")
        }
        ComparisonOperator::Gt => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} > {val_ph}")
        }
        ComparisonOperator::Ge => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("{name_placeholder} >= {val_ph}")
        }
        ComparisonOperator::Null => {
            format!("attribute_not_exists({name_placeholder})")
        }
        ComparisonOperator::NotNull => {
            format!("attribute_exists({name_placeholder})")
        }
        ComparisonOperator::Contains => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("contains({name_placeholder}, {val_ph})")
        }
        ComparisonOperator::NotContains => {
            // In the legacy API, NOT_CONTAINS fails when the attribute does
            // not exist.  The modern `NOT contains(path, :v)` expression
            // would return true for a missing attribute, so we additionally
            // require `attribute_exists(path)`.
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!(
                "(attribute_exists({name_placeholder}) AND NOT contains({name_placeholder}, \
                 {val_ph}))"
            )
        }
        ComparisonOperator::BeginsWith => {
            let val_ph = format!("{val_prefix}{counter}");
            if let Some(v) = value_list.first() {
                values.insert(val_ph.clone(), v.clone());
            }
            format!("begins_with({name_placeholder}, {val_ph})")
        }
        ComparisonOperator::In => {
            let mut in_vals = Vec::new();
            for (i, v) in value_list.iter().enumerate() {
                let val_ph = format!("{val_prefix}{counter}i{i}");
                values.insert(val_ph.clone(), v.clone());
                in_vals.push(val_ph);
            }
            format!("{name_placeholder} IN ({})", in_vals.join(", "))
        }
        ComparisonOperator::Between => {
            let low_ph = format!("{val_prefix}{counter}lo");
            let high_ph = format!("{val_prefix}{counter}hi");
            if let Some(v) = value_list.first() {
                values.insert(low_ph.clone(), v.clone());
            }
            if let Some(v) = value_list.get(1) {
                values.insert(high_ph.clone(), v.clone());
            }
            format!("{name_placeholder} BETWEEN {low_ph} AND {high_ph}")
        }
    }
}

/// Build a condition fragment for Expected conversion (uses `:lcexpv` prefix).
fn build_expected_condition_fragment(
    name_placeholder: &str,
    op: &ComparisonOperator,
    value_list: &[AttributeValue],
    values: &mut HashMap<String, AttributeValue>,
    counter: &mut usize,
) -> String {
    build_condition_fragment(name_placeholder, op, value_list, values, counter, ":lcexpv")
}

/// Merge generated expression attribute names into an existing map.
fn merge_expression_names(target: &mut HashMap<String, String>, source: HashMap<String, String>) {
    for (k, v) in source {
        target.entry(k).or_insert(v);
    }
}

/// Merge generated expression attribute values into an existing map.
fn merge_expression_values(
    target: &mut HashMap<String, AttributeValue>,
    source: HashMap<String, AttributeValue>,
) {
    for (k, v) in source {
        target.entry(k).or_insert(v);
    }
}

// ---------------------------------------------------------------------------
// Expression attribute name/value usage validation
// ---------------------------------------------------------------------------

/// Validate that expression attribute values do not contain empty sets.
///
/// DynamoDB does not allow empty sets (SS, NS, BS with zero elements).
/// Validate an `Expected` map for correctness before converting to a condition expression.
/// Validate that `ConditionalOperator` is only used when `Expected` has
/// conditions.  DynamoDB rejects `ConditionalOperator` when `Expected` is
/// missing or empty.
fn validate_conditional_operator(
    conditional_operator: Option<&ConditionalOperator>,
    expected: &HashMap<String, ExpectedAttributeValue>,
) -> Result<(), DynamoDBError> {
    if conditional_operator.is_some() && expected.is_empty() {
        return Err(DynamoDBError::validation(
            "ConditionalOperator cannot be used without Expected or with an empty Expected map",
        ));
    }
    Ok(())
}

/// Validate that `ReturnValuesOnConditionCheckFailure` has a valid value.
/// Only `NONE` and `ALL_OLD` are accepted.
fn validate_return_values_on_condition_check_failure(
    value: Option<&str>,
) -> Result<(), DynamoDBError> {
    if let Some(v) = value {
        if v != "NONE" && v != "ALL_OLD" {
            return Err(DynamoDBError::validation(format!(
                "1 validation error detected: Value '{v}' at \
                 'returnValuesOnConditionCheckFailure' failed to satisfy constraint: Member must \
                 satisfy enum value set: [NONE, ALL_OLD]"
            )));
        }
    }
    Ok(())
}

/// Detect duplicate primary keys within a batch of items.
fn detect_duplicate_keys<'a>(
    key_schema: &KeySchema,
    items: impl Iterator<Item = &'a HashMap<String, AttributeValue>>,
) -> Result<(), DynamoDBError> {
    let mut seen = HashSet::new();
    for item in items {
        if let Ok(pk) = extract_primary_key(key_schema, item) {
            if !seen.insert(pk) {
                return Err(DynamoDBError::validation(
                    "Provided list of item keys contains duplicates",
                ));
            }
        }
    }
    Ok(())
}

/// Validate the operand count and value types for a single `ComparisonOperator`.
fn validate_comparison_operator(
    comp_op: &ComparisonOperator,
    value_list: &[AttributeValue],
) -> Result<(), DynamoDBError> {
    let count = value_list.len();
    match comp_op {
        ComparisonOperator::Eq
        | ComparisonOperator::Ne
        | ComparisonOperator::Lt
        | ComparisonOperator::Le
        | ComparisonOperator::Gt
        | ComparisonOperator::Ge
        | ComparisonOperator::BeginsWith => {
            if count != 1 {
                return Err(DynamoDBError::validation(format!(
                    "One or more parameter values were invalid: Invalid number of argument(s) for \
                     the {comp_op} ComparisonOperator"
                )));
            }
        }
        ComparisonOperator::Contains | ComparisonOperator::NotContains => {
            if count != 1 {
                return Err(DynamoDBError::validation(format!(
                    "One or more parameter values were invalid: Invalid number of argument(s) for \
                     the {comp_op} ComparisonOperator"
                )));
            }
            // CONTAINS/NOT_CONTAINS only accept scalar types (S, N, B).
            if let Some(val) = value_list.first() {
                if !is_scalar_attribute_value(val) {
                    return Err(DynamoDBError::validation(format!(
                        "One or more parameter values were invalid: ComparisonOperator {comp_op} \
                         is not valid for {val_type} AttributeValue type",
                        val_type = val.type_descriptor(),
                    )));
                }
            }
        }
        ComparisonOperator::Between => {
            if count != 2 {
                return Err(DynamoDBError::validation(
                    "One or more parameter values were invalid: Invalid number of argument(s) for \
                     the BETWEEN ComparisonOperator",
                ));
            }
        }
        ComparisonOperator::In => {
            if count == 0 {
                return Err(DynamoDBError::validation(
                    "One or more parameter values were invalid: Invalid number of argument(s) for \
                     the IN ComparisonOperator",
                ));
            }
            // IN requires all values to be scalar and of the same type.
            for val in value_list {
                if !is_scalar_attribute_value(val) {
                    return Err(DynamoDBError::validation(
                        "One or more parameter values were invalid: ComparisonOperator IN is not \
                         valid for non-scalar AttributeValue type",
                    ));
                }
            }
            // All values must be the same type.
            if count > 1 {
                let first_type = value_list[0].type_descriptor();
                for val in &value_list[1..] {
                    if val.type_descriptor() != first_type {
                        return Err(DynamoDBError::validation(
                            "One or more parameter values were invalid: AttributeValues inside \
                             AttributeValueList must all be of the same type",
                        ));
                    }
                }
            }
        }
        ComparisonOperator::Null | ComparisonOperator::NotNull => {
            if count != 0 {
                return Err(DynamoDBError::validation(format!(
                    "One or more parameter values were invalid: Invalid number of argument(s) for \
                     the {comp_op} ComparisonOperator"
                )));
            }
        }
    }
    Ok(())
}

/// Validate all entries in the legacy `Expected` parameter.
fn validate_expected(
    expected: &HashMap<String, ExpectedAttributeValue>,
) -> Result<(), DynamoDBError> {
    for (attr_name, exp) in expected {
        if let Some(ref comp_op) = exp.comparison_operator {
            // When using ComparisonOperator, the legacy Value/Exists fields must not be set.
            if exp.value.is_some() || exp.exists.is_some() {
                return Err(DynamoDBError::validation(format!(
                    "One or more parameter values were invalid: Value or Exists cannot be used \
                     with ComparisonOperator for attribute ({attr_name})"
                )));
            }
            validate_comparison_operator(comp_op, &exp.attribute_value_list)?;
        } else if exp.value.is_none() && exp.exists.is_none() {
            // Must have at least one of: ComparisonOperator, Value, or Exists.
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values were invalid: Value or ComparisonOperator must be \
                 used in Expected for attribute ({attr_name})"
            )));
        } else if exp.exists == Some(true) && exp.value.is_none() {
            // Exists:True without Value is a validation error.
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values were invalid: Exists is set to TRUE for attribute \
                 ({attr_name}), Value must also be set"
            )));
        } else if exp.exists == Some(false) && exp.value.is_some() {
            // Exists:False with Value is a validation error.
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values were invalid: Value cannot be used when Exists is \
                 set to FALSE for attribute ({attr_name})"
            )));
        }
    }
    Ok(())
}

/// Check whether an `AttributeValue` is a scalar type (S, N, or B).
fn is_scalar_attribute_value(val: &AttributeValue) -> bool {
    matches!(
        val,
        AttributeValue::S(_) | AttributeValue::N(_) | AttributeValue::B(_)
    )
}

fn validate_no_empty_sets(values: &HashMap<String, AttributeValue>) -> Result<(), DynamoDBError> {
    for (key, val) in values {
        if is_empty_set(val) {
            return Err(DynamoDBError::validation(format!(
                "One or more parameter values are not valid. The AttributeValue for a member of \
                 the ExpressionAttributeValues ({key}) contains an empty set"
            )));
        }
    }
    Ok(())
}

/// Reject an empty condition expression.
fn validate_condition_not_empty(condition: Option<&str>) -> Result<(), DynamoDBError> {
    if let Some(cond) = condition {
        if cond.trim().is_empty() {
            return Err(DynamoDBError::validation(
                "Invalid ConditionExpression: The expression can not be empty;",
            ));
        }
    }
    Ok(())
}

/// Reject an empty filter expression.
fn validate_filter_not_empty(filter: Option<&str>) -> Result<(), DynamoDBError> {
    if let Some(f) = filter {
        if f.trim().is_empty() {
            return Err(DynamoDBError::validation(
                "Invalid FilterExpression: The expression can not be empty;",
            ));
        }
    }
    Ok(())
}

/// Validate that a FilterExpression does not reference any key attributes
/// (partition key or sort key). DynamoDB forbids filtering on key attributes;
/// users must use `KeyConditionExpression` instead.
fn validate_filter_no_key_attrs(
    expr: &crate::expression::Expr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
) -> Result<(), DynamoDBError> {
    let mut paths: HashSet<String> = HashSet::new();
    collect_paths_from_expr(expr, &mut paths);
    for path_name in &paths {
        // Resolve `#placeholder` references to actual attribute names.
        let resolved = if path_name.starts_with('#') {
            names.get(path_name.as_str()).map(String::as_str)
        } else {
            Some(path_name.as_str())
        };
        if let Some(attr_name) = resolved {
            if is_key_attribute(attr_name, key_schema) {
                return Err(DynamoDBError::validation(format!(
                    "Filter Expression can not contain key attribute {attr_name}",
                )));
            }
        }
    }
    Ok(())
}

/// Validate that an item's attributes do not contain empty sets.
fn validate_item_no_empty_sets(
    item: &HashMap<String, AttributeValue>,
) -> Result<(), DynamoDBError> {
    for val in item.values() {
        if contains_empty_set(val) {
            return Err(DynamoDBError::validation(
                "One or more parameter values were invalid: An number of elements of the input \
                 set is empty",
            ));
        }
    }
    Ok(())
}

/// Check if an `AttributeValue` is an empty set.
fn is_empty_set(val: &AttributeValue) -> bool {
    matches!(val, AttributeValue::Ss(v) if v.is_empty())
        || matches!(val, AttributeValue::Ns(v) if v.is_empty())
        || matches!(val, AttributeValue::Bs(v) if v.is_empty())
}

/// Check if an `AttributeValue` contains an empty set (including nested).
fn contains_empty_set(val: &AttributeValue) -> bool {
    if is_empty_set(val) {
        return true;
    }
    match val {
        AttributeValue::L(list) => list.iter().any(contains_empty_set),
        AttributeValue::M(map) => map.values().any(contains_empty_set),
        _ => false,
    }
}

/// Validate that all provided expression attribute names and values are
/// actually used in the parsed expressions. DynamoDB returns a
/// `ValidationException` if any unused names or values are present.
///
/// The `used_names` and `used_values` sets should be pre-populated by calling
/// the `collect_*` functions on all parsed expression ASTs.
fn validate_no_unused_names(
    provided_names: &HashMap<String, String>,
    used_names: &HashSet<String>,
) -> Result<(), DynamoDBError> {
    let unused: Vec<&str> = provided_names
        .keys()
        .filter(|k| !used_names.contains(k.as_str()))
        .map(String::as_str)
        .collect();
    if !unused.is_empty() {
        return Err(DynamoDBError::validation(format!(
            "Value provided in ExpressionAttributeNames unused in expressions: keys: {{{}}}",
            unused.join(", ")
        )));
    }
    Ok(())
}

/// Validate that all provided expression attribute values are used.
fn validate_no_unused_values(
    provided_values: &HashMap<String, AttributeValue>,
    used_values: &HashSet<String>,
) -> Result<(), DynamoDBError> {
    let unused: Vec<&str> = provided_values
        .keys()
        .filter(|k| !used_values.contains(k.as_str()))
        .map(String::as_str)
        .collect();
    if !unused.is_empty() {
        return Err(DynamoDBError::validation(format!(
            "Value provided in ExpressionAttributeValues unused in expressions: keys: {{{}}}",
            unused.join(", ")
        )));
    }
    Ok(())
}

/// Validate that all expression attribute name references used in expressions
/// are provided in `ExpressionAttributeNames`.
fn validate_no_unresolved_names(
    provided_names: &HashMap<String, String>,
    used_names: &HashSet<String>,
) -> Result<(), DynamoDBError> {
    for name in used_names {
        if name.starts_with('#') && !provided_names.contains_key(name.as_str()) {
            return Err(DynamoDBError::validation(format!(
                "Value provided in ExpressionAttributeNames unused in expressions: unresolved \
                 attribute name reference: {name}"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Update expression validation
// ---------------------------------------------------------------------------

/// Resolve the top-level attribute name from a path element.
fn resolve_path_top_name(path: &AttributePath, names: &HashMap<String, String>) -> Option<String> {
    let PathElement::Attribute(name) = path.elements.first()? else {
        return None;
    };
    if name.starts_with('#') {
        names.get(name.as_str()).cloned()
    } else {
        Some(name.clone())
    }
}

/// Resolve all path elements to concrete names/indices for comparison.
fn resolve_path_elements(
    path: &AttributePath,
    names: &HashMap<String, String>,
) -> Vec<ResolvedPathElement> {
    path.elements
        .iter()
        .map(|elem| match elem {
            PathElement::Attribute(name) => {
                let resolved = if name.starts_with('#') {
                    names
                        .get(name.as_str())
                        .cloned()
                        .unwrap_or_else(|| name.clone())
                } else {
                    name.clone()
                };
                ResolvedPathElement::Attribute(resolved)
            }
            PathElement::Index(idx) => ResolvedPathElement::Index(*idx),
        })
        .collect()
}

/// A resolved path element (after `#name` substitution).
#[derive(Debug, Clone, PartialEq, Eq)]
enum ResolvedPathElement {
    /// A resolved attribute name.
    Attribute(String),
    /// A list index.
    Index(usize),
}

/// Check whether two resolved paths overlap (one is a prefix of the other or they are equal).
fn paths_overlap(a: &[ResolvedPathElement], b: &[ResolvedPathElement]) -> bool {
    let min_len = a.len().min(b.len());
    for i in 0..min_len {
        if a[i] != b[i] {
            return false;
        }
    }
    true
}

/// Check whether two resolved paths conflict (one uses dot access and the other uses
/// index access at the same position).
fn paths_conflict(a: &[ResolvedPathElement], b: &[ResolvedPathElement]) -> bool {
    let min_len = a.len().min(b.len());
    for i in 0..min_len {
        match (&a[i], &b[i]) {
            // Same attribute or same index: continue
            (ResolvedPathElement::Attribute(na), ResolvedPathElement::Attribute(nb)) => {
                if na != nb {
                    return false;
                }
            }
            (ResolvedPathElement::Index(ia), ResolvedPathElement::Index(ib)) => {
                if ia != ib {
                    return false;
                }
            }
            // One uses dot, other uses index at same position: conflict
            (ResolvedPathElement::Attribute(_), ResolvedPathElement::Index(_))
            | (ResolvedPathElement::Index(_), ResolvedPathElement::Attribute(_)) => {
                return true;
            }
        }
    }
    false
}

/// Validate update expression paths: reject key attribute modifications and overlapping paths.
fn validate_update_paths(
    update: &UpdateExpr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
) -> Result<(), DynamoDBError> {
    // Collect all target paths from the update expression.
    let mut all_paths: Vec<&AttributePath> = Vec::new();
    for action in &update.set_actions {
        all_paths.push(&action.path);
    }
    for path in &update.remove_paths {
        all_paths.push(path);
    }
    for action in &update.add_actions {
        all_paths.push(&action.path);
    }
    for action in &update.delete_actions {
        all_paths.push(&action.path);
    }

    // Check that no path targets a key attribute (top-level only).
    let key_names: Vec<&str> = {
        let mut v = vec![key_schema.partition_key.name.as_str()];
        if let Some(ref sk) = key_schema.sort_key {
            v.push(sk.name.as_str());
        }
        v
    };

    for path in &all_paths {
        if let Some(top_name) = resolve_path_top_name(path, names) {
            // Only top-level paths that directly target a key attribute are rejected.
            // Nested paths like key_attr.sub_attr are not key modifications.
            if path.elements.len() == 1 && key_names.contains(&top_name.as_str()) {
                return Err(DynamoDBError::validation(format!(
                    "Cannot update attribute ({top_name}). This attribute is part of the key"
                )));
            }
        }
    }

    // Check for overlapping or conflicting paths.
    let resolved: Vec<Vec<ResolvedPathElement>> = all_paths
        .iter()
        .map(|p| resolve_path_elements(p, names))
        .collect();

    for i in 0..resolved.len() {
        for j in (i + 1)..resolved.len() {
            if paths_overlap(&resolved[i], &resolved[j]) {
                return Err(DynamoDBError::validation(
                    "Invalid UpdateExpression: Two document paths overlap with each other; must \
                     remove or rewrite one of these paths",
                ));
            }
            if paths_conflict(&resolved[i], &resolved[j]) {
                return Err(DynamoDBError::validation(
                    "Invalid UpdateExpression: Two document paths conflict with each other; must \
                     remove or rewrite one of these paths",
                ));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// ReturnValues helpers
// ---------------------------------------------------------------------------

/// Check whether an item only contains key attributes (partition key and
/// optional sort key). Used to detect REMOVE-only updates on non-existent
/// items where the result would be an item with only key attributes, which
/// DynamoDB does not store.
fn item_has_only_key_attrs(item: &HashMap<String, AttributeValue>, key_schema: &KeySchema) -> bool {
    item.keys().all(|k| {
        k == &key_schema.partition_key.name
            || key_schema.sort_key.as_ref().is_some_and(|sk| k == &sk.name)
    })
}

/// Extract only the key attributes from an item.
///
/// Used to build the `Keys` field in DynamoDB Streams change records.
fn extract_key_attributes(
    item: &HashMap<String, AttributeValue>,
    key_schema_elements: &[ruststack_dynamodb_model::types::KeySchemaElement],
) -> HashMap<String, AttributeValue> {
    key_schema_elements
        .iter()
        .filter_map(|ks| {
            item.get(&ks.attribute_name)
                .map(|v| (ks.attribute_name.clone(), v.clone()))
        })
        .collect()
}

/// Collect all target paths from an update expression (SET targets, REMOVE
/// paths, ADD targets, DELETE targets), with `#name` placeholders resolved.
fn collect_update_target_paths(
    update: &UpdateExpr,
    names: &HashMap<String, String>,
) -> Vec<Vec<ResolvedPathElement>> {
    let mut paths = Vec::new();
    for action in &update.set_actions {
        paths.push(resolve_path_elements(&action.path, names));
    }
    for path in &update.remove_paths {
        paths.push(resolve_path_elements(path, names));
    }
    for action in &update.add_actions {
        paths.push(resolve_path_elements(&action.path, names));
    }
    for action in &update.delete_actions {
        paths.push(resolve_path_elements(&action.path, names));
    }
    paths
}

/// Look up the value at a resolved path within an item. Returns `None` if
/// the path does not exist in the item.
fn lookup_resolved_path_value(
    item: &HashMap<String, AttributeValue>,
    path: &[ResolvedPathElement],
) -> Option<AttributeValue> {
    if path.is_empty() {
        return None;
    }

    let mut current: Option<&AttributeValue> = None;
    for (i, elem) in path.iter().enumerate() {
        match elem {
            ResolvedPathElement::Attribute(name) => {
                if i == 0 {
                    current = item.get(name);
                } else {
                    let map = current?.as_m()?;
                    current = map.get(name);
                }
            }
            ResolvedPathElement::Index(idx) => {
                let list = current?.as_l()?;
                current = list.get(*idx);
            }
        }
    }

    current.cloned()
}

/// Insert a value into a result map at a given resolved path, reconstructing
/// the nested structure. For nested paths like `[Attribute("a"),
/// Attribute("b")]` with value `"hi"`, this produces `{"a": {"b": "hi"}}`.
///
/// For list index paths like `[Attribute("a"), Attribute("c"), Index(1)]`
/// with value `2`, this produces `{"a": {"c": [2]}}` (a single-element list
/// containing just the value at that index).
///
/// When multiple paths share a common prefix (e.g., `a.b` and `a.c[1]`),
/// the results are merged into a single nested structure.
fn insert_at_resolved_path(
    result: &mut HashMap<String, AttributeValue>,
    path: &[ResolvedPathElement],
    value: AttributeValue,
) {
    if path.is_empty() {
        return;
    }

    // The first element must be an Attribute (top-level key).
    let ResolvedPathElement::Attribute(top_key) = &path[0] else {
        return;
    };

    if path.len() == 1 {
        // Simple top-level attribute: just insert directly.
        result.insert(top_key.clone(), value);
        return;
    }

    // For nested paths, wrap the value in the appropriate nested structure
    // and merge with any existing content at the same path.
    let nested_value = wrap_value_in_nested_path(&path[1..], value);

    match result.get_mut(top_key) {
        Some(existing) => {
            merge_attribute_values(existing, nested_value);
        }
        None => {
            result.insert(top_key.clone(), nested_value);
        }
    }
}

/// Wrap a value in nested Map/List structure according to the remaining path
/// elements. For example, path `[Attribute("b")]` with value `"hi"` produces
/// `M({"b": "hi"})`. Path `[Attribute("c"), Index(1)]` with value `2`
/// produces `M({"c": L([2])})`.
fn wrap_value_in_nested_path(
    path: &[ResolvedPathElement],
    value: AttributeValue,
) -> AttributeValue {
    if path.is_empty() {
        return value;
    }

    match &path[0] {
        ResolvedPathElement::Attribute(name) => {
            let inner = wrap_value_in_nested_path(&path[1..], value);
            let mut map = HashMap::new();
            map.insert(name.clone(), inner);
            AttributeValue::M(map)
        }
        ResolvedPathElement::Index(_) => {
            // For list indices, wrap the leaf value in a single-element list.
            // DynamoDB returns just the single affected element in
            // UPDATED_OLD / UPDATED_NEW for list index paths.
            let inner = wrap_value_in_nested_path(&path[1..], value);
            AttributeValue::L(vec![inner])
        }
    }
}

/// Deep-merge `source` into `target`. When both are maps, merge recursively.
/// When both are lists, concatenate. Otherwise, `source` overwrites `target`.
fn merge_attribute_values(target: &mut AttributeValue, source: AttributeValue) {
    match (target, source) {
        (AttributeValue::M(target_map), AttributeValue::M(source_map)) => {
            for (key, source_val) in source_map {
                match target_map.get_mut(&key) {
                    Some(existing) => merge_attribute_values(existing, source_val),
                    None => {
                        target_map.insert(key, source_val);
                    }
                }
            }
        }
        (AttributeValue::L(target_list), AttributeValue::L(source_list)) => {
            target_list.extend(source_list);
        }
        (target, source) => {
            *target = source;
        }
    }
}

/// Compute the return value attributes for `UpdateItem` based on the
/// `ReturnValues` parameter.
///
/// - `NONE` / `None`: empty map
/// - `ALL_OLD`: all attributes of the old item (or empty if no old item)
/// - `ALL_NEW`: all attributes of the new item
/// - `UPDATED_OLD`: for each path targeted by the update expression, return the old value if it
///   existed (before the update). Only returns the specific nested sub-path, not the entire
///   top-level attribute.
/// - `UPDATED_NEW`: for each path targeted by the update expression, return the new value if it
///   exists (after the update). For REMOVE'd attributes, the path no longer exists so it is not
///   returned.
fn compute_update_return_values(
    return_values: Option<&ReturnValue>,
    old_item: Option<&HashMap<String, AttributeValue>>,
    new_item: &HashMap<String, AttributeValue>,
    key_schema: &KeySchema,
    parsed_update: Option<&UpdateExpr>,
    expression_names: &HashMap<String, String>,
) -> HashMap<String, AttributeValue> {
    match return_values {
        Some(ReturnValue::AllOld) => old_item.cloned().unwrap_or_default(),
        Some(ReturnValue::AllNew) => new_item.clone(),
        Some(ReturnValue::UpdatedOld) => {
            let Some(old) = old_item else {
                return HashMap::new();
            };
            let Some(update) = parsed_update else {
                return HashMap::new();
            };
            let paths = collect_update_target_paths(update, expression_names);
            let mut result = HashMap::new();
            for path in &paths {
                // Skip key attributes (the first element of the path is the
                // top-level attribute name).
                if let Some(ResolvedPathElement::Attribute(name)) = path.first() {
                    if is_key_attribute(name, key_schema) {
                        continue;
                    }
                }
                // Look up the old value at this exact path.
                if let Some(old_val) = lookup_resolved_path_value(old, path) {
                    insert_at_resolved_path(&mut result, path, old_val);
                }
            }
            result
        }
        Some(ReturnValue::UpdatedNew) => {
            let Some(update) = parsed_update else {
                return HashMap::new();
            };
            let paths = collect_update_target_paths(update, expression_names);
            let mut result = HashMap::new();
            for path in &paths {
                // Skip key attributes.
                if let Some(ResolvedPathElement::Attribute(name)) = path.first() {
                    if is_key_attribute(name, key_schema) {
                        continue;
                    }
                }
                // Look up the new value at this exact path. REMOVE'd paths
                // will not exist in the new item, so they are naturally
                // excluded.
                if let Some(new_val) = lookup_resolved_path_value(new_item, path) {
                    insert_at_resolved_path(&mut result, path, new_val);
                }
            }
            result
        }
        _ => HashMap::new(),
    }
}

/// Check if an attribute name is a key attribute (partition or sort key).
fn is_key_attribute(name: &str, key_schema: &KeySchema) -> bool {
    name == key_schema.partition_key.name
        || key_schema
            .sort_key
            .as_ref()
            .is_some_and(|sk| name == sk.name)
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// CreateTable validation helpers
// ---------------------------------------------------------------------------

/// Validate `KeySchema` structure: exactly 1 HASH, at most 1 RANGE, max 2 elements.
fn validate_key_schema_structure(
    elements: &[ruststack_dynamodb_model::types::KeySchemaElement],
) -> Result<(), DynamoDBError> {
    let hash_count = elements
        .iter()
        .filter(|e| e.key_type == KeyType::Hash)
        .count();
    let range_count = elements
        .iter()
        .filter(|e| e.key_type == KeyType::Range)
        .count();

    if hash_count != 1 {
        return Err(DynamoDBError::validation(
            "Invalid KeySchema: Some index key schema element is not valid",
        ));
    }
    if range_count > 1 {
        return Err(DynamoDBError::validation(
            "Too many KeySchema elements; expected at most 2",
        ));
    }
    if elements.len() > 2 {
        return Err(DynamoDBError::validation(
            "Too many KeySchema elements; expected at most 2",
        ));
    }
    Ok(())
}

/// Validate `AttributeDefinitions` for duplicates.
fn validate_attribute_definitions(
    definitions: &[AttributeDefinition],
) -> Result<(), DynamoDBError> {
    let mut seen = HashSet::new();
    for def in definitions {
        if !seen.insert(&def.attribute_name) {
            return Err(DynamoDBError::validation(format!(
                "Duplicate AttributeName in AttributeDefinitions: {}",
                def.attribute_name,
            )));
        }
    }
    Ok(())
}

/// Validate billing mode: unknown values rejected, consistency between billing
/// mode and provisioned throughput.
fn validate_billing_mode(
    billing_mode: Option<&BillingMode>,
    provisioned_throughput: Option<&ruststack_dynamodb_model::types::ProvisionedThroughput>,
) -> Result<BillingMode, DynamoDBError> {
    let mode = billing_mode.cloned().unwrap_or(BillingMode::Provisioned);
    match &mode {
        BillingMode::Unknown(val) => {
            return Err(DynamoDBError::validation(format!(
                "1 validation error detected: Value '{val}' at 'billingMode' failed to satisfy \
                 constraint: Member must satisfy enum value set: [PROVISIONED, PAY_PER_REQUEST]"
            )));
        }
        BillingMode::PayPerRequest => {
            if provisioned_throughput.is_some() {
                return Err(DynamoDBError::validation(
                    "One or more parameter values were invalid: Neither ReadCapacityUnits nor \
                     WriteCapacityUnits can be specified when BillingMode is PAY_PER_REQUEST",
                ));
            }
        }
        BillingMode::Provisioned => {
            if provisioned_throughput.is_none() {
                return Err(DynamoDBError::validation(
                    "No provisioned throughput specified for the table",
                ));
            }
        }
    }
    Ok(mode)
}

/// Validate that all defined attributes are used as keys in the table or indexes.
fn validate_no_spurious_attribute_definitions(
    definitions: &[AttributeDefinition],
    key_schema: &[ruststack_dynamodb_model::types::KeySchemaElement],
    gsi_definitions: &[ruststack_dynamodb_model::types::GlobalSecondaryIndex],
    lsi_definitions: &[ruststack_dynamodb_model::types::LocalSecondaryIndex],
) -> Result<(), DynamoDBError> {
    let mut used_attrs = HashSet::new();
    for elem in key_schema {
        used_attrs.insert(elem.attribute_name.as_str());
    }
    for gsi in gsi_definitions {
        for elem in &gsi.key_schema {
            used_attrs.insert(elem.attribute_name.as_str());
        }
    }
    for lsi in lsi_definitions {
        for elem in &lsi.key_schema {
            used_attrs.insert(elem.attribute_name.as_str());
        }
    }

    let spurious: Vec<&str> = definitions
        .iter()
        .filter(|d| !used_attrs.contains(d.attribute_name.as_str()))
        .map(|d| d.attribute_name.as_str())
        .collect();

    if !spurious.is_empty() {
        return Err(DynamoDBError::validation(
            "Number of attributes in AttributeDefinitions does not exactly match number of \
             attributes in KeySchema and secondary indexes",
        ));
    }
    Ok(())
}

/// Parse key schema elements and attribute definitions into a `KeySchema`.
fn parse_key_schema(
    elements: &[ruststack_dynamodb_model::types::KeySchemaElement],
    definitions: &[AttributeDefinition],
) -> Result<KeySchema, DynamoDBError> {
    let mut partition_key = None;
    let mut sort_key = None;

    for elem in elements {
        match elem.key_type {
            KeyType::Hash => partition_key = Some(elem.attribute_name.clone()),
            KeyType::Range => sort_key = Some(elem.attribute_name.clone()),
        }
    }

    let pk_name = partition_key
        .ok_or_else(|| DynamoDBError::validation("Key schema must contain a HASH key element"))?;

    let pk_type = find_attribute_type(definitions, &pk_name)?;
    validate_key_attribute_type(&pk_type, &pk_name)?;

    let sk_attr = if let Some(ref sk) = sort_key {
        let sk_type = find_attribute_type(definitions, sk)?;
        validate_key_attribute_type(&sk_type, sk)?;
        Some(KeyAttribute {
            name: sk.clone(),
            attr_type: sk_type,
        })
    } else {
        None
    };

    Ok(KeySchema {
        partition_key: KeyAttribute {
            name: pk_name,
            attr_type: pk_type,
        },
        sort_key: sk_attr,
    })
}

/// Validate that a key attribute type is one of the allowed types (S, N, B).
fn validate_key_attribute_type(
    attr_type: &ScalarAttributeType,
    _attr_name: &str,
) -> Result<(), DynamoDBError> {
    if attr_type.is_valid_key_type() {
        Ok(())
    } else {
        Err(DynamoDBError::validation(format!(
            "Member must satisfy enum value set: [S, N, B], got '{attr_type}'"
        )))
    }
}

fn find_attribute_type(
    definitions: &[AttributeDefinition],
    name: &str,
) -> Result<ScalarAttributeType, DynamoDBError> {
    definitions
        .iter()
        .find(|d| d.attribute_name == name)
        .map(|d| d.attribute_type.clone())
        .ok_or_else(|| {
            DynamoDBError::validation(format!(
                "One or more parameter values were invalid: Some index key schema elements are \
                 not valid. The following index key schema element does not have a matching \
                 AttributeDefinition: {name}"
            ))
        })
}

/// Resolve an attribute path operand to its real name (resolving `#name` references).
///
/// Returns `Some((resolved_name, is_nested))` if the operand is a path, or `None` for
/// value references.
fn resolve_kce_path_name(
    operand: &crate::expression::ast::Operand,
    names: &HashMap<String, String>,
) -> Option<(String, bool)> {
    if let crate::expression::ast::Operand::Path(path) = operand {
        if path.elements.len() > 1 {
            return Some(("__nested__".to_owned(), true));
        }
        if let Some(crate::expression::ast::PathElement::Attribute(name)) = path.elements.first() {
            let resolved = if name.starts_with('#') {
                names
                    .get(name.as_str())
                    .cloned()
                    .unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            return Some((resolved, false));
        }
    }
    None
}

/// Validate a resolved path name is a key attribute, rejecting nested paths.
fn validate_kce_path_is_key(
    resolved: &str,
    nested: bool,
    key_schema: &KeySchema,
) -> Result<(), DynamoDBError> {
    if nested {
        return Err(DynamoDBError::validation(
            "Key condition expression does not support nested attribute paths",
        ));
    }
    if !is_key_attribute(resolved, key_schema) {
        return Err(DynamoDBError::validation(format!(
            "Query condition missed key schema element: {}",
            key_schema.partition_key.name,
        )));
    }
    Ok(())
}

/// Recursively count key attribute references in a key condition expression,
/// rejecting illegal constructs (OR, NOT, IN, `<>`, non-key attributes, etc.).
fn collect_kce_key_refs(
    expr: &crate::expression::Expr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
    pk_count: &mut u32,
    sk_count: &mut u32,
) -> Result<(), DynamoDBError> {
    use crate::expression::ast::{CompareOp, Expr, FunctionName, LogicalOp};

    match expr {
        Expr::Compare { left, op, right } => {
            // Both sides: check path references are key attributes.
            for operand in [left.as_ref(), right.as_ref()] {
                if let Some((resolved, nested)) = resolve_kce_path_name(operand, names) {
                    validate_kce_path_is_key(&resolved, nested, key_schema)?;
                }
            }
            // Count which key attribute this condition references.
            for operand in [left.as_ref(), right.as_ref()] {
                if let Some((resolved, _)) = resolve_kce_path_name(operand, names) {
                    if resolved == key_schema.partition_key.name {
                        if *op != CompareOp::Eq {
                            return Err(DynamoDBError::validation(
                                "Query key condition not supported",
                            ));
                        }
                        *pk_count += 1;
                    } else if key_schema
                        .sort_key
                        .as_ref()
                        .is_some_and(|sk| sk.name == resolved)
                    {
                        if *op == CompareOp::Ne {
                            return Err(DynamoDBError::validation(
                                "Unsupported operator on KeyConditionExpression: operator: <>",
                            ));
                        }
                        *sk_count += 1;
                    }
                }
            }
            Ok(())
        }
        Expr::Between { value, .. } => {
            if let Some((resolved, nested)) = resolve_kce_path_name(value, names) {
                validate_kce_path_is_key(&resolved, nested, key_schema)?;
                if resolved == key_schema.partition_key.name {
                    return Err(DynamoDBError::validation(
                        "Query key condition not supported",
                    ));
                }
                *sk_count += 1;
            }
            Ok(())
        }
        Expr::In { .. } => Err(DynamoDBError::validation(
            "Unsupported operator on KeyConditionExpression: operator: IN",
        )),
        Expr::Logical { op, left, right } => {
            if *op == LogicalOp::Or {
                return Err(DynamoDBError::validation(
                    "Unsupported operator in KeyConditionExpression: OR",
                ));
            }
            collect_kce_key_refs(left, key_schema, names, pk_count, sk_count)?;
            collect_kce_key_refs(right, key_schema, names, pk_count, sk_count)
        }
        Expr::Not(_) => Err(DynamoDBError::validation(
            "Unsupported operator in KeyConditionExpression: NOT",
        )),
        Expr::Function { name, args, .. } => match name {
            FunctionName::BeginsWith => {
                if let Some(first_arg) = args.first() {
                    if let Some((resolved, nested)) = resolve_kce_path_name(first_arg, names) {
                        validate_kce_path_is_key(&resolved, nested, key_schema)?;
                        if resolved == key_schema.partition_key.name {
                            return Err(DynamoDBError::validation(
                                "Query key condition not supported",
                            ));
                        }
                        *sk_count += 1;
                    }
                }
                Ok(())
            }
            other => Err(DynamoDBError::validation(format!(
                "Unsupported function in KeyConditionExpression: {other}",
            ))),
        },
    }
}

/// Validate a parsed key condition expression.
///
/// Rejects patterns not allowed in `KeyConditionExpression`:
/// - OR operator, NOT operator
/// - `<>` (not-equal) and `IN` operators
/// - Non-key attributes, nested attribute paths
/// - Functions other than `begins_with`
/// - Multiple conditions on the same key attribute
/// - Non-equality conditions on the partition key
fn validate_key_condition_expr(
    expr: &crate::expression::Expr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
) -> Result<(), DynamoDBError> {
    let mut pk_count = 0u32;
    let mut sk_count = 0u32;
    collect_kce_key_refs(expr, key_schema, names, &mut pk_count, &mut sk_count)?;

    if pk_count == 0 {
        return Err(DynamoDBError::validation(format!(
            "Query condition missed key schema element: {}",
            key_schema.partition_key.name,
        )));
    }
    if pk_count > 1 || sk_count > 1 {
        return Err(DynamoDBError::validation(
            "KeyConditionExpressions must only contain one condition per key",
        ));
    }
    Ok(())
}

/// Extract partition key value and optional sort key condition from a parsed
/// key condition expression. This is a simplified extraction that handles the
/// common patterns: `pk = :val` and `pk = :val AND sk <op> :val2`.
///
/// Also validates that the value types match the key schema types.
fn extract_key_condition(
    expr: &crate::expression::Expr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
    values: &HashMap<String, AttributeValue>,
) -> Result<(AttributeValue, Option<SortKeyCondition>), DynamoDBError> {
    use crate::expression::ast::{CompareOp, Expr, LogicalOp};

    match expr {
        // Simple: pk = :val
        Expr::Compare {
            left,
            op: CompareOp::Eq,
            right,
        } => {
            let pk_val =
                resolve_key_value(left, right, &key_schema.partition_key.name, names, values)?;
            validate_key_value_type(&pk_val, &key_schema.partition_key)?;
            Ok((pk_val, None))
        }
        // AND: pk = :val AND sk <op> :val2
        Expr::Logical {
            op: LogicalOp::And,
            left,
            right,
        } => {
            // Try left as partition, right as sort.
            if let Ok((pk_val, _)) = extract_key_condition(left, key_schema, names, values) {
                let sort_cond = extract_sort_condition(right, key_schema, names, values)?;
                if let (Some(sk), Some(cond)) = (&key_schema.sort_key, &sort_cond) {
                    validate_sort_condition_type(cond, sk)?;
                }
                return Ok((pk_val, sort_cond));
            }
            // Try right as partition, left as sort.
            if let Ok((pk_val, _)) = extract_key_condition(right, key_schema, names, values) {
                let sort_cond = extract_sort_condition(left, key_schema, names, values)?;
                if let (Some(sk), Some(cond)) = (&key_schema.sort_key, &sort_cond) {
                    validate_sort_condition_type(cond, sk)?;
                }
                return Ok((pk_val, sort_cond));
            }
            Err(DynamoDBError::validation(
                "KeyConditionExpression must contain an equality condition on the partition key",
            ))
        }
        _ => Err(DynamoDBError::validation(
            "KeyConditionExpression must contain an equality condition on the partition key",
        )),
    }
}

/// Validate that the provided attribute value matches the expected key attribute type.
fn validate_key_value_type(
    value: &AttributeValue,
    key_attr: &KeyAttribute,
) -> Result<(), DynamoDBError> {
    if !matches!(
        (&key_attr.attr_type, value),
        (ScalarAttributeType::S, AttributeValue::S(_))
            | (ScalarAttributeType::N, AttributeValue::N(_))
            | (ScalarAttributeType::B, AttributeValue::B(_))
    ) {
        return Err(DynamoDBError::validation(format!(
            "Condition parameter type does not match schema type for key attribute '{}'",
            key_attr.name,
        )));
    }
    Ok(())
}

/// Validate that the sort key condition values match the expected sort key type.
fn validate_sort_condition_type(
    cond: &SortKeyCondition,
    sk: &KeyAttribute,
) -> Result<(), DynamoDBError> {
    let validate = |sort_val: &SortableAttributeValue| -> Result<(), DynamoDBError> {
        if !matches!(
            (&sk.attr_type, sort_val),
            (ScalarAttributeType::S, SortableAttributeValue::S(_))
                | (ScalarAttributeType::N, SortableAttributeValue::N(_))
                | (ScalarAttributeType::B, SortableAttributeValue::B(_))
        ) {
            return Err(DynamoDBError::validation(format!(
                "Condition parameter type does not match schema type for key attribute '{}'",
                sk.name,
            )));
        }
        Ok(())
    };

    match cond {
        SortKeyCondition::Eq(v)
        | SortKeyCondition::Lt(v)
        | SortKeyCondition::Le(v)
        | SortKeyCondition::Gt(v)
        | SortKeyCondition::Ge(v) => validate(v),
        SortKeyCondition::Between(low, high) => {
            validate(low)?;
            validate(high)
        }
        SortKeyCondition::BeginsWithStr(_) | SortKeyCondition::BeginsWithBytes(_) => {
            // begins_with type checking is handled elsewhere (string/binary only)
            Ok(())
        }
    }
}

/// Extract a sort key condition from an expression node.
///
/// Handles reversed comparisons: `:val < key` is equivalent to `key > :val`.
fn extract_sort_condition(
    expr: &crate::expression::Expr,
    key_schema: &KeySchema,
    names: &HashMap<String, String>,
    values: &HashMap<String, AttributeValue>,
) -> Result<Option<SortKeyCondition>, DynamoDBError> {
    use crate::expression::ast::{CompareOp, Expr, FunctionName};

    let sk_name = match &key_schema.sort_key {
        Some(sk) => &sk.name,
        None => return Ok(None),
    };

    match expr {
        Expr::Compare { left, op, right } => {
            // Determine whether the key path is on the left or right side.
            // If reversed (value on left, key on right), flip the operator.
            let (val, effective_op) =
                resolve_sort_value_with_direction(left, right, sk_name, names, values, *op)?;
            let sortable = to_sortable(sk_name, &val)?;
            let cond = match effective_op {
                CompareOp::Eq => SortKeyCondition::Eq(sortable),
                CompareOp::Lt => SortKeyCondition::Lt(sortable),
                CompareOp::Le => SortKeyCondition::Le(sortable),
                CompareOp::Gt => SortKeyCondition::Gt(sortable),
                CompareOp::Ge => SortKeyCondition::Ge(sortable),
                CompareOp::Ne => {
                    return Err(DynamoDBError::validation(
                        "Sort key condition does not support <> operator",
                    ));
                }
            };
            Ok(Some(cond))
        }
        Expr::Between {
            value: _,
            low,
            high,
        } => {
            let low_val = resolve_operand_value(low, names, values)?;
            let high_val = resolve_operand_value(high, names, values)?;
            let low_s = to_sortable(sk_name, &low_val)?;
            let high_s = to_sortable(sk_name, &high_val)?;
            Ok(Some(SortKeyCondition::Between(low_s, high_s)))
        }
        Expr::Function {
            name: FunctionName::BeginsWith,
            args,
        } if args.len() == 2 => {
            let prefix_val = resolve_operand_value(&args[1], names, values)?;
            match prefix_val {
                AttributeValue::S(s) => Ok(Some(SortKeyCondition::BeginsWithStr(s))),
                AttributeValue::B(b) => Ok(Some(SortKeyCondition::BeginsWithBytes(b))),
                _ => Err(DynamoDBError::validation(
                    "begins_with requires a string or binary argument",
                )),
            }
        }
        _ => Ok(None),
    }
}

/// Resolve an operand to an `AttributeValue`.
fn resolve_operand_value(
    operand: &crate::expression::ast::Operand,
    _names: &HashMap<String, String>,
    values: &HashMap<String, AttributeValue>,
) -> Result<AttributeValue, DynamoDBError> {
    use crate::expression::ast::Operand;
    match operand {
        Operand::Value(name) => {
            let key = format!(":{name}");
            values.get(&key).cloned().ok_or_else(|| {
                DynamoDBError::validation(format!(
                    "Value {key} not found in ExpressionAttributeValues"
                ))
            })
        }
        _ => Err(DynamoDBError::validation(
            "Expected a value reference (:value) in key condition",
        )),
    }
}

/// Resolve a key equality condition: one side should be the key path, the other
/// a value reference.
fn resolve_key_value(
    left: &crate::expression::ast::Operand,
    right: &crate::expression::ast::Operand,
    key_name: &str,
    names: &HashMap<String, String>,
    values: &HashMap<String, AttributeValue>,
) -> Result<AttributeValue, DynamoDBError> {
    use crate::expression::ast::Operand;

    let is_key_path = |op: &Operand| -> bool {
        if let Operand::Path(path) = op {
            if path.elements.len() == 1 {
                if let crate::expression::ast::PathElement::Attribute(name) = &path.elements[0] {
                    let resolved = if name.starts_with('#') {
                        names
                            .get(name.as_str())
                            .map_or(name.as_str(), String::as_str)
                    } else {
                        name.as_str()
                    };
                    return resolved == key_name;
                }
            }
        }
        false
    };

    if is_key_path(left) {
        resolve_operand_value(right, names, values)
    } else if is_key_path(right) {
        resolve_operand_value(left, names, values)
    } else {
        Err(DynamoDBError::validation(format!(
            "KeyConditionExpression must reference key attribute '{key_name}'"
        )))
    }
}

/// Resolve a sort key comparison, detecting whether the key path is on the
/// left or right side. If the key is on the right (reversed comparison like
/// `:val < key`), the operator is flipped (`<` becomes `>`, etc.).
fn resolve_sort_value_with_direction(
    left: &crate::expression::ast::Operand,
    right: &crate::expression::ast::Operand,
    key_name: &str,
    names: &HashMap<String, String>,
    values: &HashMap<String, AttributeValue>,
    op: crate::expression::ast::CompareOp,
) -> Result<(AttributeValue, crate::expression::ast::CompareOp), DynamoDBError> {
    use crate::expression::ast::{CompareOp, Operand};

    let is_key_path = |operand: &Operand| -> bool {
        if let Operand::Path(path) = operand {
            if path.elements.len() == 1 {
                if let crate::expression::ast::PathElement::Attribute(name) = &path.elements[0] {
                    let resolved = if name.starts_with('#') {
                        names
                            .get(name.as_str())
                            .map_or(name.as_str(), String::as_str)
                    } else {
                        name.as_str()
                    };
                    return resolved == key_name;
                }
            }
        }
        false
    };

    if is_key_path(left) {
        // Normal order: key <op> value
        let val = resolve_operand_value(right, names, values)?;
        Ok((val, op))
    } else if is_key_path(right) {
        // Reversed: value <op> key -> key <flipped_op> value
        let val = resolve_operand_value(left, names, values)?;
        let flipped = match op {
            CompareOp::Eq => CompareOp::Eq,
            CompareOp::Ne => CompareOp::Ne,
            CompareOp::Lt => CompareOp::Gt,
            CompareOp::Le => CompareOp::Ge,
            CompareOp::Gt => CompareOp::Lt,
            CompareOp::Ge => CompareOp::Le,
        };
        Ok((val, flipped))
    } else {
        Err(DynamoDBError::validation(format!(
            "KeyConditionExpression must reference key attribute '{key_name}'"
        )))
    }
}

/// Convert an `AttributeValue` to a `SortableAttributeValue` for key conditions.
fn to_sortable(
    attr_name: &str,
    val: &AttributeValue,
) -> Result<SortableAttributeValue, DynamoDBError> {
    SortableAttributeValue::from_attribute_value(attr_name, val).map_err(storage_error_to_dynamodb)
}

/// Build a `last_evaluated_key` map from partition and optional sort key.
fn build_last_evaluated_key(
    key_schema: &KeySchema,
    partition: &AttributeValue,
    sort: Option<&AttributeValue>,
) -> HashMap<String, AttributeValue> {
    let mut key = HashMap::new();
    key.insert(key_schema.partition_key.name.clone(), partition.clone());
    if let (Some(sk), Some(sv)) = (&key_schema.sort_key, sort) {
        key.insert(sk.name.clone(), sv.clone());
    }
    key
}

/// Maximum response size for a single Query/Scan response (1 MB).
const MAX_RESPONSE_BYTES: u64 = 1_048_576;

/// Result type for GSI query operations.
type GsiQueryResult = (
    Vec<HashMap<String, AttributeValue>>,
    Option<HashMap<String, AttributeValue>>,
);

/// Execute a query against a Global Secondary Index.
///
/// Because GSI items are not stored in a separate index structure, this
/// function scans all items in the table's primary storage, filters by the
/// GSI partition key, applies the sort condition on the GSI sort key, sorts
/// the results, and applies pagination/limit.
///
/// Returns `(items, last_evaluated_key)` where `last_evaluated_key` contains
/// both the GSI keys and the table's primary keys (matching DynamoDB
/// behaviour for GSI query responses).
fn query_gsi(
    table: &DynamoDBTable,
    gsi_key_schema: &KeySchema,
    partition_value: &AttributeValue,
    sort_condition: Option<&SortKeyCondition>,
    scan_forward: bool,
    limit: Option<usize>,
    exclusive_start_key: &HashMap<String, AttributeValue>,
) -> GsiQueryResult {
    let gsi_pk_name = &gsi_key_schema.partition_key.name;

    // Scan all items, filter by GSI partition key, apply sort condition, and sort.
    let mut matching = gsi_filter_and_sort(
        table,
        gsi_key_schema,
        gsi_pk_name,
        partition_value,
        sort_condition,
        scan_forward,
    );

    // Apply exclusive_start_key pagination.
    if !exclusive_start_key.is_empty() {
        gsi_apply_pagination(
            &mut matching,
            exclusive_start_key,
            gsi_pk_name,
            gsi_key_schema,
            &table.key_schema,
        );
    }

    // Apply limit and 1 MB size cap.
    let (selected, has_more) = gsi_apply_limit(matching, limit);

    // Build last_evaluated_key containing both GSI keys and table primary keys.
    let last_evaluated_key = if has_more {
        selected
            .last()
            .map(|last_item| gsi_build_last_key(last_item, gsi_key_schema, &table.key_schema))
    } else {
        None
    };

    (selected, last_evaluated_key)
}

/// Scan, filter by GSI partition key, apply sort condition, and sort items.
fn gsi_filter_and_sort(
    table: &DynamoDBTable,
    gsi_key_schema: &KeySchema,
    gsi_pk_name: &str,
    partition_value: &AttributeValue,
    sort_condition: Option<&SortKeyCondition>,
    scan_forward: bool,
) -> Vec<HashMap<String, AttributeValue>> {
    let (all_items, _) = table.storage.scan(None, None, None, None);

    let mut matching: Vec<HashMap<String, AttributeValue>> = all_items
        .into_iter()
        .filter(|item| item.get(gsi_pk_name).is_some_and(|v| v == partition_value))
        .collect();

    // Apply GSI sort key condition if present.
    if let Some(sk_def) = &gsi_key_schema.sort_key {
        if let Some(condition) = sort_condition {
            matching.retain(|item| {
                item.get(&sk_def.name)
                    .and_then(|v| {
                        SortableAttributeValue::from_attribute_value(&sk_def.name, v).ok()
                    })
                    .is_some_and(|sv| condition.matches(&sv))
            });
        }
        // Sort by the GSI sort key.
        matching.sort_by(|a, b| {
            let a_sk = a
                .get(&sk_def.name)
                .and_then(|v| SortableAttributeValue::from_attribute_value(&sk_def.name, v).ok());
            let b_sk = b
                .get(&sk_def.name)
                .and_then(|v| SortableAttributeValue::from_attribute_value(&sk_def.name, v).ok());
            match (a_sk, b_sk) {
                (Some(av), Some(bv)) => av.cmp(&bv),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            }
        });
    }

    if !scan_forward {
        matching.reverse();
    }

    matching
}

/// Apply exclusive_start_key pagination to GSI query results.
///
/// Removes all items up to and including the item that matches the start key.
fn gsi_apply_pagination(
    items: &mut Vec<HashMap<String, AttributeValue>>,
    start_key: &HashMap<String, AttributeValue>,
    gsi_pk_name: &str,
    gsi_key_schema: &KeySchema,
    table_key_schema: &KeySchema,
) {
    // An item matches the start key when every key component present in
    // `start_key` has an equal value in the item.
    let matches_start = |item: &HashMap<String, AttributeValue>| -> bool {
        start_key.iter().all(|(key_name, expected)| {
            // For sort key attributes, compare via SortableAttributeValue to
            // get correct DynamoDB ordering semantics.
            let is_sort_attr = gsi_key_schema
                .sort_key
                .as_ref()
                .is_some_and(|sk| sk.name == *key_name)
                || table_key_schema
                    .sort_key
                    .as_ref()
                    .is_some_and(|sk| sk.name == *key_name);

            if is_sort_attr {
                item.get(key_name).is_some_and(|v| v == expected)
            } else {
                // Partition key: direct equality.
                item.get(key_name).is_some_and(|v| v == expected)
            }
        })
    };

    // Find the position of the start key item, then keep only items after it.
    let skip_pos = items.iter().position(|item| {
        // Must match GSI PK at minimum.
        item.get(gsi_pk_name).is_some_and(|v| {
            start_key
                .get(gsi_pk_name)
                .is_some_and(|expected| v == expected)
        }) && matches_start(item)
    });

    if let Some(pos) = skip_pos {
        // Remove everything up to and including the matched item.
        items.drain(..=pos);
    } else {
        // Start key not found -- this shouldn't normally happen, but if it
        // does, return no items (consistent with DynamoDB).
        items.clear();
    }
}

/// Apply count limit and 1 MB size cap, returning `(selected_items, has_more)`.
fn gsi_apply_limit(
    items: Vec<HashMap<String, AttributeValue>>,
    limit: Option<usize>,
) -> (Vec<HashMap<String, AttributeValue>>, bool) {
    let effective_limit = limit.unwrap_or(usize::MAX);
    let mut selected: Vec<HashMap<String, AttributeValue>> = Vec::new();
    let mut cumulative_size: u64 = 0;

    for item in items {
        if selected.len() >= effective_limit {
            return (selected, true);
        }
        let item_size = calculate_item_size(&item);
        if !selected.is_empty() && cumulative_size + item_size > MAX_RESPONSE_BYTES {
            return (selected, true);
        }
        cumulative_size += item_size;
        selected.push(item);
    }

    (selected, false)
}

/// Build the `last_evaluated_key` for a GSI query response.
///
/// DynamoDB includes both the GSI keys and the table's primary keys in the
/// last evaluated key for GSI query responses.
fn gsi_build_last_key(
    last_item: &HashMap<String, AttributeValue>,
    gsi_key_schema: &KeySchema,
    table_key_schema: &KeySchema,
) -> HashMap<String, AttributeValue> {
    let mut key = HashMap::new();

    // Include GSI partition key.
    if let Some(v) = last_item.get(&gsi_key_schema.partition_key.name) {
        key.insert(gsi_key_schema.partition_key.name.clone(), v.clone());
    }
    // Include GSI sort key.
    if let Some(sk_def) = &gsi_key_schema.sort_key {
        if let Some(v) = last_item.get(&sk_def.name) {
            key.insert(sk_def.name.clone(), v.clone());
        }
    }
    // Include table primary partition key.
    if let Some(v) = last_item.get(&table_key_schema.partition_key.name) {
        key.insert(table_key_schema.partition_key.name.clone(), v.clone());
    }
    // Include table primary sort key.
    if let Some(sk_def) = &table_key_schema.sort_key {
        if let Some(v) = last_item.get(&sk_def.name) {
            key.insert(sk_def.name.clone(), v.clone());
        }
    }

    key
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use ruststack_dynamodb_model::{
        error::DynamoDBErrorCode,
        input::{BatchWriteItemInput, CreateTableInput, UpdateItemInput},
        types::{
            AttributeDefinition, KeySchemaElement, KeyType, PutRequest, ScalarAttributeType,
            WriteRequest,
        },
    };

    use super::*;

    /// Create a provider with a pre-configured test table named "TestTable".
    fn setup_provider_with_table() -> RustStackDynamoDB {
        let provider = RustStackDynamoDB::new(DynamoDBConfig::default());
        let input = CreateTableInput {
            table_name: "TestTable".to_owned(),
            key_schema: vec![KeySchemaElement {
                attribute_name: "pk".to_owned(),
                key_type: KeyType::Hash,
            }],
            attribute_definitions: vec![AttributeDefinition {
                attribute_name: "pk".to_owned(),
                attribute_type: ScalarAttributeType::S,
            }],
            billing_mode: Some(BillingMode::PayPerRequest),
            ..Default::default()
        };
        provider.handle_create_table(input).unwrap();
        provider
    }

    #[test]
    fn test_should_allow_update_item_without_update_expression() {
        // UpdateItem without update_expression should create the item from key.
        let provider = setup_provider_with_table();
        let input = UpdateItemInput {
            table_name: "TestTable".to_owned(),
            key: HashMap::from([("pk".to_owned(), AttributeValue::S("k1".to_owned()))]),
            update_expression: None,
            ..Default::default()
        };
        let result = provider.handle_update_item(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_error_on_batch_write_item_exceeding_25_items() {
        let provider = setup_provider_with_table();

        let mut writes = Vec::new();
        for i in 0..26 {
            writes.push(WriteRequest {
                put_request: Some(PutRequest {
                    item: HashMap::from([("pk".to_owned(), AttributeValue::S(format!("key{i}")))]),
                }),
                delete_request: None,
            });
        }

        let input = BatchWriteItemInput {
            request_items: HashMap::from([("TestTable".to_owned(), writes)]),
            return_consumed_capacity: None,
            return_item_collection_metrics: None,
        };
        let result = provider.handle_batch_write_item(input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, DynamoDBErrorCode::ValidationException);
        assert!(err.message.contains("exceeds the limit"));
    }

    #[test]
    fn test_should_allow_batch_write_item_at_exactly_25_items() {
        let provider = setup_provider_with_table();

        let writes: Vec<WriteRequest> = (0..25)
            .map(|i| WriteRequest {
                put_request: Some(PutRequest {
                    item: HashMap::from([("pk".to_owned(), AttributeValue::S(format!("key{i}")))]),
                }),
                delete_request: None,
            })
            .collect();

        let input = BatchWriteItemInput {
            request_items: HashMap::from([("TestTable".to_owned(), writes)]),
            return_consumed_capacity: None,
            return_item_collection_metrics: None,
        };
        let result = provider.handle_batch_write_item(input);
        assert!(result.is_ok());
    }

    #[test]
    fn test_should_resolve_operand_value_with_colon_prefix() {
        // Verify that resolve_operand_value correctly prepends ":" when
        // looking up values in the expression attribute values map.
        use crate::expression::ast::Operand;

        let values = HashMap::from([(":myval".to_owned(), AttributeValue::S("hello".to_owned()))]);
        let names = HashMap::new();

        // The parser stores Operand::Value("myval") without the ":" prefix.
        let operand = Operand::Value("myval".to_owned());
        let result = resolve_operand_value(&operand, &names, &values);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), AttributeValue::S("hello".to_owned()));
    }

    #[test]
    fn test_should_error_resolve_operand_value_missing_prefix() {
        // Verify that looking up a value not in the map produces an error.
        use crate::expression::ast::Operand;

        let values = HashMap::new();
        let names = HashMap::new();

        let operand = Operand::Value("missing".to_owned());
        let result = resolve_operand_value(&operand, &names, &values);
        assert!(result.is_err());
    }

    #[test]
    fn test_should_convert_conditions_eq() {
        let conditions = HashMap::from([(
            "age".to_owned(),
            Condition {
                comparison_operator: ComparisonOperator::Eq,
                attribute_value_list: vec![AttributeValue::N("25".to_owned())],
            },
        )]);
        let (expr, names, values) =
            convert_conditions_to_expression(&conditions, None, "#lcattr", ":lcval");
        assert!(expr.contains('='));
        assert!(names.values().any(|v| v == "age"));
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn test_should_convert_conditions_between() {
        let conditions = HashMap::from([(
            "score".to_owned(),
            Condition {
                comparison_operator: ComparisonOperator::Between,
                attribute_value_list: vec![
                    AttributeValue::N("10".to_owned()),
                    AttributeValue::N("90".to_owned()),
                ],
            },
        )]);
        let (expr, names, values) =
            convert_conditions_to_expression(&conditions, None, "#lcattr", ":lcval");
        assert!(expr.contains("BETWEEN"));
        assert!(expr.contains("AND"));
        assert!(names.values().any(|v| v == "score"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_should_convert_conditions_in() {
        let conditions = HashMap::from([(
            "status".to_owned(),
            Condition {
                comparison_operator: ComparisonOperator::In,
                attribute_value_list: vec![
                    AttributeValue::S("active".to_owned()),
                    AttributeValue::S("pending".to_owned()),
                ],
            },
        )]);
        let (expr, names, values) =
            convert_conditions_to_expression(&conditions, None, "#lcattr", ":lcval");
        assert!(expr.contains("IN"));
        assert!(names.values().any(|v| v == "status"));
        assert_eq!(values.len(), 2);
    }

    #[test]
    fn test_should_convert_expected_exists_false() {
        let expected = HashMap::from([(
            "email".to_owned(),
            ExpectedAttributeValue {
                exists: Some(false),
                ..Default::default()
            },
        )]);
        let (expr, names, _values) = convert_expected_to_condition(&expected, None);
        assert!(expr.contains("attribute_not_exists"));
        assert!(names.values().any(|v| v == "email"));
    }

    #[test]
    fn test_should_convert_expected_value_equality() {
        let expected = HashMap::from([(
            "name".to_owned(),
            ExpectedAttributeValue {
                value: Some(AttributeValue::S("Alice".to_owned())),
                ..Default::default()
            },
        )]);
        let (expr, names, values) = convert_expected_to_condition(&expected, None);
        assert!(expr.contains('='));
        assert!(names.values().any(|v| v == "name"));
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn test_should_convert_attribute_updates_set_and_remove() {
        let updates = HashMap::from([
            (
                "name".to_owned(),
                AttributeValueUpdate {
                    value: Some(AttributeValue::S("Bob".to_owned())),
                    action: Some(AttributeAction::Put),
                },
            ),
            (
                "old_field".to_owned(),
                AttributeValueUpdate {
                    value: None,
                    action: Some(AttributeAction::Delete),
                },
            ),
        ]);
        let (expr, names, values) = convert_attribute_updates_to_expression(&updates);
        assert!(expr.contains("SET"));
        assert!(expr.contains("REMOVE"));
        assert!(names.values().any(|v| v == "name"));
        assert!(names.values().any(|v| v == "old_field"));
        assert_eq!(values.len(), 1); // Only SET has a value
    }

    #[test]
    fn test_should_convert_attributes_to_get() {
        let attrs = vec!["id".to_owned(), "name".to_owned(), "email".to_owned()];
        let result = convert_attributes_to_get(&attrs);
        assert_eq!(result, "id, name, email");
    }

    #[test]
    fn test_should_return_none_for_missing_get_item() {
        let provider = setup_provider_with_table();
        let input = GetItemInput {
            table_name: "TestTable".to_owned(),
            key: HashMap::from([("pk".to_owned(), AttributeValue::S("nonexistent".to_owned()))]),
            ..Default::default()
        };
        let result = provider.handle_get_item(input).unwrap();
        assert!(result.item.is_none());
    }

    #[test]
    fn test_should_delete_table_with_deleting_status() {
        let provider = setup_provider_with_table();
        let input = DeleteTableInput {
            table_name: "TestTable".to_owned(),
        };
        let result = provider.handle_delete_table(input).unwrap();
        let desc = result.table_description.unwrap();
        assert_eq!(desc.table_status, Some(TableStatus::Deleting));
    }

    #[test]
    fn test_should_reject_invalid_return_values_for_put_item() {
        let provider = setup_provider_with_table();
        let input = PutItemInput {
            table_name: "TestTable".to_owned(),
            item: HashMap::from([("pk".to_owned(), AttributeValue::S("k1".to_owned()))]),
            return_values: Some(ruststack_dynamodb_model::types::ReturnValue::AllNew),
            ..Default::default()
        };
        let result = provider.handle_put_item(input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, DynamoDBErrorCode::ValidationException);
    }

    #[test]
    fn test_should_reject_invalid_return_values_for_delete_item() {
        let provider = setup_provider_with_table();
        let input = DeleteItemInput {
            table_name: "TestTable".to_owned(),
            key: HashMap::from([("pk".to_owned(), AttributeValue::S("k1".to_owned()))]),
            return_values: Some(ruststack_dynamodb_model::types::ReturnValue::AllNew),
            ..Default::default()
        };
        let result = provider.handle_delete_item(input);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, DynamoDBErrorCode::ValidationException);
    }

    #[test]
    fn test_should_handle_update_table() {
        let provider = setup_provider_with_table();
        let input = UpdateTableInput {
            table_name: "TestTable".to_owned(),
            ..Default::default()
        };
        let result = provider.handle_update_table(input);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.table_description.is_some());
    }

    #[test]
    fn test_should_compute_update_return_values_updated_old() {
        let key_schema = KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: None,
        };
        let old = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            ("name".to_owned(), AttributeValue::S("Alice".to_owned())),
            ("age".to_owned(), AttributeValue::N("30".to_owned())),
        ]);
        let new = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            ("name".to_owned(), AttributeValue::S("Bob".to_owned())),
            ("age".to_owned(), AttributeValue::N("30".to_owned())),
            (
                "email".to_owned(),
                AttributeValue::S("bob@example.com".to_owned()),
            ),
        ]);
        // Simulate: SET name = :val1, email = :val2
        let update = parse_update("SET #n = :val1, email = :val2").unwrap();
        let names = HashMap::from([("#n".to_owned(), "name".to_owned())]);
        let changed = compute_update_return_values(
            Some(&ReturnValue::UpdatedOld),
            Some(&old),
            &new,
            &key_schema,
            Some(&update),
            &names,
        );
        // "name" was targeted by SET: old was Alice, so it should be in result.
        assert_eq!(
            changed.get("name"),
            Some(&AttributeValue::S("Alice".to_owned()))
        );
        // "email" was targeted by SET but didn't exist in old, so not in
        // UPDATED_OLD result.
        assert!(!changed.contains_key("email"));
        // "pk" is not targeted by the update expression.
        assert!(!changed.contains_key("pk"));
        // "age" is not targeted by the update expression.
        assert!(!changed.contains_key("age"));
    }

    #[test]
    fn test_should_compute_update_return_values_updated_new() {
        let key_schema = KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: None,
        };
        let old = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            ("name".to_owned(), AttributeValue::S("Alice".to_owned())),
        ]);
        let new = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            ("name".to_owned(), AttributeValue::S("Bob".to_owned())),
            (
                "email".to_owned(),
                AttributeValue::S("bob@example.com".to_owned()),
            ),
        ]);
        // Simulate: SET name = :val1, email = :val2
        let update = parse_update("SET #n = :val1, email = :val2").unwrap();
        let names = HashMap::from([("#n".to_owned(), "name".to_owned())]);
        let changed = compute_update_return_values(
            Some(&ReturnValue::UpdatedNew),
            Some(&old),
            &new,
            &key_schema,
            Some(&update),
            &names,
        );
        // "name" was targeted by SET: new value is Bob.
        assert_eq!(
            changed.get("name"),
            Some(&AttributeValue::S("Bob".to_owned()))
        );
        // "email" was targeted by SET: new value in the new item.
        assert_eq!(
            changed.get("email"),
            Some(&AttributeValue::S("bob@example.com".to_owned()))
        );
        // "pk" is not targeted by the update expression.
        assert!(!changed.contains_key("pk"));
    }

    #[test]
    fn test_should_compute_update_return_values_nested_paths() {
        let key_schema = KeySchema {
            partition_key: KeyAttribute {
                name: "pk".to_owned(),
                attr_type: ScalarAttributeType::S,
            },
            sort_key: None,
        };
        let old = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            (
                "a".to_owned(),
                AttributeValue::M(HashMap::from([
                    ("b".to_owned(), AttributeValue::S("dog".to_owned())),
                    (
                        "c".to_owned(),
                        AttributeValue::L(vec![
                            AttributeValue::N("1".to_owned()),
                            AttributeValue::N("2".to_owned()),
                            AttributeValue::N("3".to_owned()),
                        ]),
                    ),
                ])),
            ),
        ]);
        let new = HashMap::from([
            ("pk".to_owned(), AttributeValue::S("k1".to_owned())),
            (
                "a".to_owned(),
                AttributeValue::M(HashMap::from([
                    ("b".to_owned(), AttributeValue::S("hi".to_owned())),
                    (
                        "c".to_owned(),
                        AttributeValue::L(vec![
                            AttributeValue::N("1".to_owned()),
                            AttributeValue::N("2".to_owned()),
                            AttributeValue::N("3".to_owned()),
                        ]),
                    ),
                ])),
            ),
        ]);
        // Simulate: SET a.b = :val
        let update = parse_update("SET a.b = :val").unwrap();
        let names = HashMap::new();
        let changed = compute_update_return_values(
            Some(&ReturnValue::UpdatedOld),
            Some(&old),
            &new,
            &key_schema,
            Some(&update),
            &names,
        );
        // Should return only the nested path a.b, not the entire a attribute.
        assert_eq!(
            changed,
            HashMap::from([(
                "a".to_owned(),
                AttributeValue::M(HashMap::from([(
                    "b".to_owned(),
                    AttributeValue::S("dog".to_owned()),
                )])),
            )])
        );

        // Test UPDATED_NEW for the same scenario.
        let changed = compute_update_return_values(
            Some(&ReturnValue::UpdatedNew),
            Some(&old),
            &new,
            &key_schema,
            Some(&update),
            &names,
        );
        assert_eq!(
            changed,
            HashMap::from([(
                "a".to_owned(),
                AttributeValue::M(HashMap::from([(
                    "b".to_owned(),
                    AttributeValue::S("hi".to_owned()),
                )])),
            )])
        );
    }
}
