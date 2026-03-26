//! IAM handler implementation bridging HTTP to business logic.
//!
//! Parses form-urlencoded request bodies, dispatches to the provider,
//! and serializes XML responses following the awsQuery protocol.
//!
//! Covers all four phases: users/roles/policies, groups/instance profiles,
//! policy versions/inline policies, and tagging/service-linked roles.

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;
use ruststack_iam_http::{
    body::IamResponseBody, dispatch::IamHandler, request::parse_form_params, response::xml_response,
};
use ruststack_iam_model::{error::IamError, operations::IamOperation};

use crate::provider::RustStackIam;

/// Handler that bridges the HTTP layer to the IAM provider.
#[derive(Debug)]
pub struct RustStackIamHandler {
    provider: Arc<RustStackIam>,
}

impl RustStackIamHandler {
    /// Create a new handler wrapping a provider.
    #[must_use]
    pub fn new(provider: Arc<RustStackIam>) -> Self {
        Self { provider }
    }
}

impl IamHandler for RustStackIamHandler {
    fn handle_operation(
        &self,
        op: IamOperation,
        body: Bytes,
    ) -> Pin<Box<dyn Future<Output = Result<http::Response<IamResponseBody>, IamError>> + Send>>
    {
        let provider = Arc::clone(&self.provider);
        Box::pin(async move { dispatch(&provider, op, &body) })
    }
}

/// Dispatch an IAM operation to the appropriate provider method.
#[allow(clippy::too_many_lines)]
fn dispatch(
    provider: &RustStackIam,
    op: IamOperation,
    body: &[u8],
) -> Result<http::Response<IamResponseBody>, IamError> {
    let params = parse_form_params(body);

    let (xml, request_id) = match op {
        // Phase 0: Users
        IamOperation::CreateUser => provider.create_user(&params)?,
        IamOperation::GetUser => provider.get_user(&params)?,
        IamOperation::DeleteUser => provider.delete_user(&params)?,
        IamOperation::ListUsers => provider.list_users(&params)?,
        IamOperation::UpdateUser => provider.update_user(&params)?,

        // Phase 0: Roles
        IamOperation::CreateRole => provider.create_role(&params)?,
        IamOperation::GetRole => provider.get_role(&params)?,
        IamOperation::DeleteRole => provider.delete_role(&params)?,
        IamOperation::ListRoles => provider.list_roles(&params)?,
        IamOperation::UpdateRole => provider.update_role(&params)?,

        // Phase 0: Managed Policies
        IamOperation::CreatePolicy => provider.create_policy(&params)?,
        IamOperation::GetPolicy => provider.get_policy(&params)?,
        IamOperation::DeletePolicy => provider.delete_policy(&params)?,
        IamOperation::ListPolicies => provider.list_policies(&params)?,

        // Phase 0: User Policy Attachment
        IamOperation::AttachUserPolicy => provider.attach_user_policy(&params)?,
        IamOperation::DetachUserPolicy => provider.detach_user_policy(&params)?,
        IamOperation::ListAttachedUserPolicies => provider.list_attached_user_policies(&params)?,

        // Phase 0: Role Policy Attachment
        IamOperation::AttachRolePolicy => provider.attach_role_policy(&params)?,
        IamOperation::DetachRolePolicy => provider.detach_role_policy(&params)?,
        IamOperation::ListAttachedRolePolicies => provider.list_attached_role_policies(&params)?,

        // Phase 0: Access Keys
        IamOperation::CreateAccessKey => provider.create_access_key(&params)?,
        IamOperation::DeleteAccessKey => provider.delete_access_key(&params)?,
        IamOperation::ListAccessKeys => provider.list_access_keys(&params)?,
        IamOperation::UpdateAccessKey => provider.update_access_key(&params)?,
        IamOperation::GetAccessKeyLastUsed => provider.get_access_key_last_used(&params)?,

        // Phase 1: Groups
        IamOperation::CreateGroup => provider.create_group(&params)?,
        IamOperation::GetGroup => provider.get_group(&params)?,
        IamOperation::DeleteGroup => provider.delete_group(&params)?,
        IamOperation::ListGroups => provider.list_groups(&params)?,
        IamOperation::UpdateGroup => provider.update_group(&params)?,

        // Phase 1: Group Membership
        IamOperation::AddUserToGroup => provider.add_user_to_group(&params)?,
        IamOperation::RemoveUserFromGroup => provider.remove_user_from_group(&params)?,
        IamOperation::ListGroupsForUser => provider.list_groups_for_user(&params)?,

        // Phase 1: Group Policy Attachment
        IamOperation::AttachGroupPolicy => provider.attach_group_policy(&params)?,
        IamOperation::DetachGroupPolicy => provider.detach_group_policy(&params)?,
        IamOperation::ListAttachedGroupPolicies => {
            provider.list_attached_group_policies(&params)?
        }

        // Phase 1: Instance Profiles
        IamOperation::CreateInstanceProfile => provider.create_instance_profile(&params)?,
        IamOperation::GetInstanceProfile => provider.get_instance_profile(&params)?,
        IamOperation::DeleteInstanceProfile => provider.delete_instance_profile(&params)?,
        IamOperation::ListInstanceProfiles => provider.list_instance_profiles(&params)?,
        IamOperation::ListInstanceProfilesForRole => {
            provider.list_instance_profiles_for_role(&params)?
        }
        IamOperation::AddRoleToInstanceProfile => provider.add_role_to_instance_profile(&params)?,
        IamOperation::RemoveRoleFromInstanceProfile => {
            provider.remove_role_from_instance_profile(&params)?
        }

        // Phase 2: Policy Versions
        IamOperation::CreatePolicyVersion => provider.create_policy_version(&params)?,
        IamOperation::GetPolicyVersion => provider.get_policy_version(&params)?,
        IamOperation::DeletePolicyVersion => provider.delete_policy_version(&params)?,
        IamOperation::ListPolicyVersions => provider.list_policy_versions(&params)?,
        IamOperation::SetDefaultPolicyVersion => provider.set_default_policy_version(&params)?,

        // Phase 2: Inline User Policies
        IamOperation::PutUserPolicy => provider.put_user_policy(&params)?,
        IamOperation::GetUserPolicy => provider.get_user_policy(&params)?,
        IamOperation::DeleteUserPolicy => provider.delete_user_policy(&params)?,
        IamOperation::ListUserPolicies => provider.list_user_policies(&params)?,

        // Phase 2: Inline Role Policies
        IamOperation::PutRolePolicy => provider.put_role_policy(&params)?,
        IamOperation::GetRolePolicy => provider.get_role_policy(&params)?,
        IamOperation::DeleteRolePolicy => provider.delete_role_policy(&params)?,
        IamOperation::ListRolePolicies => provider.list_role_policies(&params)?,

        // Phase 2: Inline Group Policies
        IamOperation::PutGroupPolicy => provider.put_group_policy(&params)?,
        IamOperation::GetGroupPolicy => provider.get_group_policy(&params)?,
        IamOperation::DeleteGroupPolicy => provider.delete_group_policy(&params)?,
        IamOperation::ListGroupPolicies => provider.list_group_policies(&params)?,

        // Phase 3: User Tags
        IamOperation::TagUser => provider.tag_user(&params)?,
        IamOperation::UntagUser => provider.untag_user(&params)?,
        IamOperation::ListUserTags => provider.list_user_tags(&params)?,

        // Phase 3: Role Tags
        IamOperation::TagRole => provider.tag_role(&params)?,
        IamOperation::UntagRole => provider.untag_role(&params)?,
        IamOperation::ListRoleTags => provider.list_role_tags(&params)?,

        // Phase 3: Service-Linked Roles
        IamOperation::CreateServiceLinkedRole => provider.create_service_linked_role(&params)?,
        IamOperation::DeleteServiceLinkedRole => provider.delete_service_linked_role(&params)?,
        IamOperation::GetServiceLinkedRoleDeletionStatus => {
            provider.get_service_linked_role_deletion_status(&params)?
        }

        // Phase 3: Assume Role Policy
        IamOperation::UpdateAssumeRolePolicy => provider.update_assume_role_policy(&params)?,

        // Phase 3: Simulation
        IamOperation::SimulatePrincipalPolicy => provider.simulate_principal_policy(&params)?,
        IamOperation::SimulateCustomPolicy => provider.simulate_custom_policy(&params)?,

        // Phase 3: Entities and Authorization Details
        IamOperation::ListEntitiesForPolicy => provider.list_entities_for_policy(&params)?,
        IamOperation::GetAccountAuthorizationDetails => {
            provider.get_account_authorization_details(&params)?
        }

        // Phase 4: OIDC Providers
        IamOperation::CreateOpenIDConnectProvider => {
            provider.create_open_id_connect_provider(&params)?
        }
        IamOperation::GetOpenIDConnectProvider => provider.get_open_id_connect_provider(&params)?,
        IamOperation::DeleteOpenIDConnectProvider => {
            provider.delete_open_id_connect_provider(&params)?
        }
        IamOperation::ListOpenIDConnectProviders => {
            provider.list_open_id_connect_providers(&params)?
        }

        // Phase 4: Policy Tags
        IamOperation::TagPolicy => provider.tag_policy(&params)?,
        IamOperation::UntagPolicy => provider.untag_policy(&params)?,
        IamOperation::ListPolicyTags => provider.list_policy_tags(&params)?,

        // Phase 4: Instance Profile Tags
        IamOperation::TagInstanceProfile => provider.tag_instance_profile(&params)?,
        IamOperation::UntagInstanceProfile => provider.untag_instance_profile(&params)?,
        IamOperation::ListInstanceProfileTags => provider.list_instance_profile_tags(&params)?,
    };

    Ok(xml_response(xml, &request_id))
}
