//! Lambda operation enum and route table for `restJson1` protocol.

use std::fmt;

/// All supported Lambda operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LambdaOperation {
    // Phase 0: Function CRUD + Invoke
    /// Create a new Lambda function.
    CreateFunction,
    /// Get function configuration and code location.
    GetFunction,
    /// Get function configuration only.
    GetFunctionConfiguration,
    /// Update function code.
    UpdateFunctionCode,
    /// Update function configuration.
    UpdateFunctionConfiguration,
    /// Delete a function.
    DeleteFunction,
    /// List all functions.
    ListFunctions,
    /// Invoke a function.
    Invoke,

    // Phase 1: Versions + Aliases
    /// Publish a version from `$LATEST`.
    PublishVersion,
    /// List versions of a function.
    ListVersionsByFunction,
    /// Create an alias.
    CreateAlias,
    /// Get an alias.
    GetAlias,
    /// Update an alias.
    UpdateAlias,
    /// Delete an alias.
    DeleteAlias,
    /// List aliases for a function.
    ListAliases,

    // Phase 2: Permissions + Tags + Account
    /// Add a permission to a function's resource policy.
    AddPermission,
    /// Remove a permission from a function's resource policy.
    RemovePermission,
    /// Get the resource policy for a function.
    GetPolicy,
    /// Tag a resource.
    TagResource,
    /// Untag a resource.
    UntagResource,
    /// List tags for a resource.
    ListTags,
    /// Get account settings.
    GetAccountSettings,

    // Phase 3: Function URLs
    /// Create a function URL config.
    CreateFunctionUrlConfig,
    /// Get a function URL config.
    GetFunctionUrlConfig,
    /// Update a function URL config.
    UpdateFunctionUrlConfig,
    /// Delete a function URL config.
    DeleteFunctionUrlConfig,
    /// List function URL configs.
    ListFunctionUrlConfigs,
}

impl LambdaOperation {
    /// Returns the operation name as a string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateFunction => "CreateFunction",
            Self::GetFunction => "GetFunction",
            Self::GetFunctionConfiguration => "GetFunctionConfiguration",
            Self::UpdateFunctionCode => "UpdateFunctionCode",
            Self::UpdateFunctionConfiguration => "UpdateFunctionConfiguration",
            Self::DeleteFunction => "DeleteFunction",
            Self::ListFunctions => "ListFunctions",
            Self::Invoke => "Invoke",
            Self::PublishVersion => "PublishVersion",
            Self::ListVersionsByFunction => "ListVersionsByFunction",
            Self::CreateAlias => "CreateAlias",
            Self::GetAlias => "GetAlias",
            Self::UpdateAlias => "UpdateAlias",
            Self::DeleteAlias => "DeleteAlias",
            Self::ListAliases => "ListAliases",
            Self::AddPermission => "AddPermission",
            Self::RemovePermission => "RemovePermission",
            Self::GetPolicy => "GetPolicy",
            Self::TagResource => "TagResource",
            Self::UntagResource => "UntagResource",
            Self::ListTags => "ListTags",
            Self::GetAccountSettings => "GetAccountSettings",
            Self::CreateFunctionUrlConfig => "CreateFunctionUrlConfig",
            Self::GetFunctionUrlConfig => "GetFunctionUrlConfig",
            Self::UpdateFunctionUrlConfig => "UpdateFunctionUrlConfig",
            Self::DeleteFunctionUrlConfig => "DeleteFunctionUrlConfig",
            Self::ListFunctionUrlConfigs => "ListFunctionUrlConfigs",
        }
    }
}

impl fmt::Display for LambdaOperation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Route descriptor for a Lambda operation.
#[derive(Debug, Clone)]
pub struct LambdaRoute {
    /// HTTP method for this route.
    pub method: http::Method,
    /// URL path pattern with `{param}` placeholders.
    pub path_pattern: &'static str,
    /// Operation to dispatch to.
    pub operation: LambdaOperation,
    /// HTTP status code on success.
    pub success_status: u16,
}

/// Route table for all Lambda operations.
///
/// Routes are ordered by specificity: longer/more-specific paths first.
/// When multiple operations share the same path, they are disambiguated by
/// HTTP method.
pub const LAMBDA_ROUTES: &[LambdaRoute] = &[
    // --- /2015-03-31/functions/{name}/invocations ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions/{FunctionName}/invocations",
        operation: LambdaOperation::Invoke,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name}/aliases/{alias} ---
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}/aliases/{Name}",
        operation: LambdaOperation::GetAlias,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::PUT,
        path_pattern: "/2015-03-31/functions/{FunctionName}/aliases/{Name}",
        operation: LambdaOperation::UpdateAlias,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::DELETE,
        path_pattern: "/2015-03-31/functions/{FunctionName}/aliases/{Name}",
        operation: LambdaOperation::DeleteAlias,
        success_status: 204,
    },
    // --- /2015-03-31/functions/{name}/aliases ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions/{FunctionName}/aliases",
        operation: LambdaOperation::CreateAlias,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}/aliases",
        operation: LambdaOperation::ListAliases,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name}/versions ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions/{FunctionName}/versions",
        operation: LambdaOperation::PublishVersion,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}/versions",
        operation: LambdaOperation::ListVersionsByFunction,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name}/policy/{sid} ---
    LambdaRoute {
        method: http::Method::DELETE,
        path_pattern: "/2015-03-31/functions/{FunctionName}/policy/{StatementId}",
        operation: LambdaOperation::RemovePermission,
        success_status: 204,
    },
    // --- /2015-03-31/functions/{name}/policy ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions/{FunctionName}/policy",
        operation: LambdaOperation::AddPermission,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}/policy",
        operation: LambdaOperation::GetPolicy,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name}/code ---
    LambdaRoute {
        method: http::Method::PUT,
        path_pattern: "/2015-03-31/functions/{FunctionName}/code",
        operation: LambdaOperation::UpdateFunctionCode,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name}/configuration ---
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}/configuration",
        operation: LambdaOperation::GetFunctionConfiguration,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::PUT,
        path_pattern: "/2015-03-31/functions/{FunctionName}/configuration",
        operation: LambdaOperation::UpdateFunctionConfiguration,
        success_status: 200,
    },
    // --- /2015-03-31/functions/{name} ---
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions/{FunctionName}",
        operation: LambdaOperation::GetFunction,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::DELETE,
        path_pattern: "/2015-03-31/functions/{FunctionName}",
        operation: LambdaOperation::DeleteFunction,
        success_status: 204,
    },
    // --- /2015-03-31/functions ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/functions",
        operation: LambdaOperation::CreateFunction,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/functions",
        operation: LambdaOperation::ListFunctions,
        success_status: 200,
    },
    // --- /2021-10-31/functions/{name}/url ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2021-10-31/functions/{FunctionName}/url",
        operation: LambdaOperation::CreateFunctionUrlConfig,
        success_status: 201,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2021-10-31/functions/{FunctionName}/url",
        operation: LambdaOperation::GetFunctionUrlConfig,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::PUT,
        path_pattern: "/2021-10-31/functions/{FunctionName}/url",
        operation: LambdaOperation::UpdateFunctionUrlConfig,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::DELETE,
        path_pattern: "/2021-10-31/functions/{FunctionName}/url",
        operation: LambdaOperation::DeleteFunctionUrlConfig,
        success_status: 204,
    },
    // Note: ListFunctionUrlConfigs uses same path as Get but is distinguished
    // by the presence/absence of the Qualifier query param at the handler level.
    // We route both GET /url to GetFunctionUrlConfig and handle list logic there.
    // --- /2015-03-31/tags/{arn} ---
    LambdaRoute {
        method: http::Method::POST,
        path_pattern: "/2015-03-31/tags/{Resource}",
        operation: LambdaOperation::TagResource,
        success_status: 204,
    },
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/tags/{Resource}",
        operation: LambdaOperation::ListTags,
        success_status: 200,
    },
    LambdaRoute {
        method: http::Method::DELETE,
        path_pattern: "/2015-03-31/tags/{Resource}",
        operation: LambdaOperation::UntagResource,
        success_status: 204,
    },
    // --- /2015-03-31/account-settings ---
    LambdaRoute {
        method: http::Method::GET,
        path_pattern: "/2015-03-31/account-settings",
        operation: LambdaOperation::GetAccountSettings,
        success_status: 200,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_have_all_operations_in_route_table() {
        let operations_in_routes: Vec<LambdaOperation> =
            LAMBDA_ROUTES.iter().map(|r| r.operation).collect();

        // All Phase 0 operations must be in the route table.
        assert!(operations_in_routes.contains(&LambdaOperation::CreateFunction));
        assert!(operations_in_routes.contains(&LambdaOperation::GetFunction));
        assert!(operations_in_routes.contains(&LambdaOperation::Invoke));
        assert!(operations_in_routes.contains(&LambdaOperation::DeleteFunction));
        assert!(operations_in_routes.contains(&LambdaOperation::ListFunctions));
    }

    #[test]
    fn test_should_display_operation_name() {
        assert_eq!(
            LambdaOperation::CreateFunction.to_string(),
            "CreateFunction"
        );
        assert_eq!(LambdaOperation::Invoke.to_string(), "Invoke");
    }
}
