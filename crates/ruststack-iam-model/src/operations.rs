//! Auto-generated from AWS IAM Smithy model. DO NOT EDIT.

/// All supported Iam operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IamOperation {
    /// The CreateUser operation.
    CreateUser,
    /// The GetUser operation.
    GetUser,
    /// The DeleteUser operation.
    DeleteUser,
    /// The ListUsers operation.
    ListUsers,
    /// The UpdateUser operation.
    UpdateUser,
    /// The CreateRole operation.
    CreateRole,
    /// The GetRole operation.
    GetRole,
    /// The DeleteRole operation.
    DeleteRole,
    /// The ListRoles operation.
    ListRoles,
    /// The UpdateRole operation.
    UpdateRole,
    /// The CreatePolicy operation.
    CreatePolicy,
    /// The GetPolicy operation.
    GetPolicy,
    /// The DeletePolicy operation.
    DeletePolicy,
    /// The ListPolicies operation.
    ListPolicies,
    /// The AttachUserPolicy operation.
    AttachUserPolicy,
    /// The DetachUserPolicy operation.
    DetachUserPolicy,
    /// The ListAttachedUserPolicies operation.
    ListAttachedUserPolicies,
    /// The AttachRolePolicy operation.
    AttachRolePolicy,
    /// The DetachRolePolicy operation.
    DetachRolePolicy,
    /// The ListAttachedRolePolicies operation.
    ListAttachedRolePolicies,
    /// The CreateAccessKey operation.
    CreateAccessKey,
    /// The DeleteAccessKey operation.
    DeleteAccessKey,
    /// The ListAccessKeys operation.
    ListAccessKeys,
    /// The UpdateAccessKey operation.
    UpdateAccessKey,
    /// The GetAccessKeyLastUsed operation.
    GetAccessKeyLastUsed,
    /// The CreateGroup operation.
    CreateGroup,
    /// The GetGroup operation.
    GetGroup,
    /// The DeleteGroup operation.
    DeleteGroup,
    /// The ListGroups operation.
    ListGroups,
    /// The UpdateGroup operation.
    UpdateGroup,
    /// The AddUserToGroup operation.
    AddUserToGroup,
    /// The RemoveUserFromGroup operation.
    RemoveUserFromGroup,
    /// The ListGroupsForUser operation.
    ListGroupsForUser,
    /// The AttachGroupPolicy operation.
    AttachGroupPolicy,
    /// The DetachGroupPolicy operation.
    DetachGroupPolicy,
    /// The ListAttachedGroupPolicies operation.
    ListAttachedGroupPolicies,
    /// The CreateInstanceProfile operation.
    CreateInstanceProfile,
    /// The GetInstanceProfile operation.
    GetInstanceProfile,
    /// The DeleteInstanceProfile operation.
    DeleteInstanceProfile,
    /// The ListInstanceProfiles operation.
    ListInstanceProfiles,
    /// The ListInstanceProfilesForRole operation.
    ListInstanceProfilesForRole,
    /// The AddRoleToInstanceProfile operation.
    AddRoleToInstanceProfile,
    /// The RemoveRoleFromInstanceProfile operation.
    RemoveRoleFromInstanceProfile,
    /// The CreatePolicyVersion operation.
    CreatePolicyVersion,
    /// The GetPolicyVersion operation.
    GetPolicyVersion,
    /// The DeletePolicyVersion operation.
    DeletePolicyVersion,
    /// The ListPolicyVersions operation.
    ListPolicyVersions,
    /// The SetDefaultPolicyVersion operation.
    SetDefaultPolicyVersion,
    /// The PutUserPolicy operation.
    PutUserPolicy,
    /// The GetUserPolicy operation.
    GetUserPolicy,
    /// The DeleteUserPolicy operation.
    DeleteUserPolicy,
    /// The ListUserPolicies operation.
    ListUserPolicies,
    /// The PutRolePolicy operation.
    PutRolePolicy,
    /// The GetRolePolicy operation.
    GetRolePolicy,
    /// The DeleteRolePolicy operation.
    DeleteRolePolicy,
    /// The ListRolePolicies operation.
    ListRolePolicies,
    /// The PutGroupPolicy operation.
    PutGroupPolicy,
    /// The GetGroupPolicy operation.
    GetGroupPolicy,
    /// The DeleteGroupPolicy operation.
    DeleteGroupPolicy,
    /// The ListGroupPolicies operation.
    ListGroupPolicies,
    /// The TagUser operation.
    TagUser,
    /// The UntagUser operation.
    UntagUser,
    /// The ListUserTags operation.
    ListUserTags,
    /// The TagRole operation.
    TagRole,
    /// The UntagRole operation.
    UntagRole,
    /// The ListRoleTags operation.
    ListRoleTags,
    /// The CreateServiceLinkedRole operation.
    CreateServiceLinkedRole,
    /// The DeleteServiceLinkedRole operation.
    DeleteServiceLinkedRole,
    /// The GetServiceLinkedRoleDeletionStatus operation.
    GetServiceLinkedRoleDeletionStatus,
    /// The UpdateAssumeRolePolicy operation.
    UpdateAssumeRolePolicy,
    /// The SimulatePrincipalPolicy operation.
    SimulatePrincipalPolicy,
    /// The SimulateCustomPolicy operation.
    SimulateCustomPolicy,
    /// The ListEntitiesForPolicy operation.
    ListEntitiesForPolicy,
    /// The GetAccountAuthorizationDetails operation.
    GetAccountAuthorizationDetails,
    /// The CreateOpenIDConnectProvider operation.
    CreateOpenIDConnectProvider,
    /// The GetOpenIDConnectProvider operation.
    GetOpenIDConnectProvider,
    /// The DeleteOpenIDConnectProvider operation.
    DeleteOpenIDConnectProvider,
    /// The ListOpenIDConnectProviders operation.
    ListOpenIDConnectProviders,
    /// The TagPolicy operation.
    TagPolicy,
    /// The UntagPolicy operation.
    UntagPolicy,
    /// The ListPolicyTags operation.
    ListPolicyTags,
    /// The TagInstanceProfile operation.
    TagInstanceProfile,
    /// The UntagInstanceProfile operation.
    UntagInstanceProfile,
    /// The ListInstanceProfileTags operation.
    ListInstanceProfileTags,
}

impl IamOperation {
    /// Returns the AWS operation name string.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CreateUser => "CreateUser",
            Self::GetUser => "GetUser",
            Self::DeleteUser => "DeleteUser",
            Self::ListUsers => "ListUsers",
            Self::UpdateUser => "UpdateUser",
            Self::CreateRole => "CreateRole",
            Self::GetRole => "GetRole",
            Self::DeleteRole => "DeleteRole",
            Self::ListRoles => "ListRoles",
            Self::UpdateRole => "UpdateRole",
            Self::CreatePolicy => "CreatePolicy",
            Self::GetPolicy => "GetPolicy",
            Self::DeletePolicy => "DeletePolicy",
            Self::ListPolicies => "ListPolicies",
            Self::AttachUserPolicy => "AttachUserPolicy",
            Self::DetachUserPolicy => "DetachUserPolicy",
            Self::ListAttachedUserPolicies => "ListAttachedUserPolicies",
            Self::AttachRolePolicy => "AttachRolePolicy",
            Self::DetachRolePolicy => "DetachRolePolicy",
            Self::ListAttachedRolePolicies => "ListAttachedRolePolicies",
            Self::CreateAccessKey => "CreateAccessKey",
            Self::DeleteAccessKey => "DeleteAccessKey",
            Self::ListAccessKeys => "ListAccessKeys",
            Self::UpdateAccessKey => "UpdateAccessKey",
            Self::GetAccessKeyLastUsed => "GetAccessKeyLastUsed",
            Self::CreateGroup => "CreateGroup",
            Self::GetGroup => "GetGroup",
            Self::DeleteGroup => "DeleteGroup",
            Self::ListGroups => "ListGroups",
            Self::UpdateGroup => "UpdateGroup",
            Self::AddUserToGroup => "AddUserToGroup",
            Self::RemoveUserFromGroup => "RemoveUserFromGroup",
            Self::ListGroupsForUser => "ListGroupsForUser",
            Self::AttachGroupPolicy => "AttachGroupPolicy",
            Self::DetachGroupPolicy => "DetachGroupPolicy",
            Self::ListAttachedGroupPolicies => "ListAttachedGroupPolicies",
            Self::CreateInstanceProfile => "CreateInstanceProfile",
            Self::GetInstanceProfile => "GetInstanceProfile",
            Self::DeleteInstanceProfile => "DeleteInstanceProfile",
            Self::ListInstanceProfiles => "ListInstanceProfiles",
            Self::ListInstanceProfilesForRole => "ListInstanceProfilesForRole",
            Self::AddRoleToInstanceProfile => "AddRoleToInstanceProfile",
            Self::RemoveRoleFromInstanceProfile => "RemoveRoleFromInstanceProfile",
            Self::CreatePolicyVersion => "CreatePolicyVersion",
            Self::GetPolicyVersion => "GetPolicyVersion",
            Self::DeletePolicyVersion => "DeletePolicyVersion",
            Self::ListPolicyVersions => "ListPolicyVersions",
            Self::SetDefaultPolicyVersion => "SetDefaultPolicyVersion",
            Self::PutUserPolicy => "PutUserPolicy",
            Self::GetUserPolicy => "GetUserPolicy",
            Self::DeleteUserPolicy => "DeleteUserPolicy",
            Self::ListUserPolicies => "ListUserPolicies",
            Self::PutRolePolicy => "PutRolePolicy",
            Self::GetRolePolicy => "GetRolePolicy",
            Self::DeleteRolePolicy => "DeleteRolePolicy",
            Self::ListRolePolicies => "ListRolePolicies",
            Self::PutGroupPolicy => "PutGroupPolicy",
            Self::GetGroupPolicy => "GetGroupPolicy",
            Self::DeleteGroupPolicy => "DeleteGroupPolicy",
            Self::ListGroupPolicies => "ListGroupPolicies",
            Self::TagUser => "TagUser",
            Self::UntagUser => "UntagUser",
            Self::ListUserTags => "ListUserTags",
            Self::TagRole => "TagRole",
            Self::UntagRole => "UntagRole",
            Self::ListRoleTags => "ListRoleTags",
            Self::CreateServiceLinkedRole => "CreateServiceLinkedRole",
            Self::DeleteServiceLinkedRole => "DeleteServiceLinkedRole",
            Self::GetServiceLinkedRoleDeletionStatus => "GetServiceLinkedRoleDeletionStatus",
            Self::UpdateAssumeRolePolicy => "UpdateAssumeRolePolicy",
            Self::SimulatePrincipalPolicy => "SimulatePrincipalPolicy",
            Self::SimulateCustomPolicy => "SimulateCustomPolicy",
            Self::ListEntitiesForPolicy => "ListEntitiesForPolicy",
            Self::GetAccountAuthorizationDetails => "GetAccountAuthorizationDetails",
            Self::CreateOpenIDConnectProvider => "CreateOpenIDConnectProvider",
            Self::GetOpenIDConnectProvider => "GetOpenIDConnectProvider",
            Self::DeleteOpenIDConnectProvider => "DeleteOpenIDConnectProvider",
            Self::ListOpenIDConnectProviders => "ListOpenIDConnectProviders",
            Self::TagPolicy => "TagPolicy",
            Self::UntagPolicy => "UntagPolicy",
            Self::ListPolicyTags => "ListPolicyTags",
            Self::TagInstanceProfile => "TagInstanceProfile",
            Self::UntagInstanceProfile => "UntagInstanceProfile",
            Self::ListInstanceProfileTags => "ListInstanceProfileTags",
        }
    }

    /// Parse an operation name string into an IamOperation.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "CreateUser" => Some(Self::CreateUser),
            "GetUser" => Some(Self::GetUser),
            "DeleteUser" => Some(Self::DeleteUser),
            "ListUsers" => Some(Self::ListUsers),
            "UpdateUser" => Some(Self::UpdateUser),
            "CreateRole" => Some(Self::CreateRole),
            "GetRole" => Some(Self::GetRole),
            "DeleteRole" => Some(Self::DeleteRole),
            "ListRoles" => Some(Self::ListRoles),
            "UpdateRole" => Some(Self::UpdateRole),
            "CreatePolicy" => Some(Self::CreatePolicy),
            "GetPolicy" => Some(Self::GetPolicy),
            "DeletePolicy" => Some(Self::DeletePolicy),
            "ListPolicies" => Some(Self::ListPolicies),
            "AttachUserPolicy" => Some(Self::AttachUserPolicy),
            "DetachUserPolicy" => Some(Self::DetachUserPolicy),
            "ListAttachedUserPolicies" => Some(Self::ListAttachedUserPolicies),
            "AttachRolePolicy" => Some(Self::AttachRolePolicy),
            "DetachRolePolicy" => Some(Self::DetachRolePolicy),
            "ListAttachedRolePolicies" => Some(Self::ListAttachedRolePolicies),
            "CreateAccessKey" => Some(Self::CreateAccessKey),
            "DeleteAccessKey" => Some(Self::DeleteAccessKey),
            "ListAccessKeys" => Some(Self::ListAccessKeys),
            "UpdateAccessKey" => Some(Self::UpdateAccessKey),
            "GetAccessKeyLastUsed" => Some(Self::GetAccessKeyLastUsed),
            "CreateGroup" => Some(Self::CreateGroup),
            "GetGroup" => Some(Self::GetGroup),
            "DeleteGroup" => Some(Self::DeleteGroup),
            "ListGroups" => Some(Self::ListGroups),
            "UpdateGroup" => Some(Self::UpdateGroup),
            "AddUserToGroup" => Some(Self::AddUserToGroup),
            "RemoveUserFromGroup" => Some(Self::RemoveUserFromGroup),
            "ListGroupsForUser" => Some(Self::ListGroupsForUser),
            "AttachGroupPolicy" => Some(Self::AttachGroupPolicy),
            "DetachGroupPolicy" => Some(Self::DetachGroupPolicy),
            "ListAttachedGroupPolicies" => Some(Self::ListAttachedGroupPolicies),
            "CreateInstanceProfile" => Some(Self::CreateInstanceProfile),
            "GetInstanceProfile" => Some(Self::GetInstanceProfile),
            "DeleteInstanceProfile" => Some(Self::DeleteInstanceProfile),
            "ListInstanceProfiles" => Some(Self::ListInstanceProfiles),
            "ListInstanceProfilesForRole" => Some(Self::ListInstanceProfilesForRole),
            "AddRoleToInstanceProfile" => Some(Self::AddRoleToInstanceProfile),
            "RemoveRoleFromInstanceProfile" => Some(Self::RemoveRoleFromInstanceProfile),
            "CreatePolicyVersion" => Some(Self::CreatePolicyVersion),
            "GetPolicyVersion" => Some(Self::GetPolicyVersion),
            "DeletePolicyVersion" => Some(Self::DeletePolicyVersion),
            "ListPolicyVersions" => Some(Self::ListPolicyVersions),
            "SetDefaultPolicyVersion" => Some(Self::SetDefaultPolicyVersion),
            "PutUserPolicy" => Some(Self::PutUserPolicy),
            "GetUserPolicy" => Some(Self::GetUserPolicy),
            "DeleteUserPolicy" => Some(Self::DeleteUserPolicy),
            "ListUserPolicies" => Some(Self::ListUserPolicies),
            "PutRolePolicy" => Some(Self::PutRolePolicy),
            "GetRolePolicy" => Some(Self::GetRolePolicy),
            "DeleteRolePolicy" => Some(Self::DeleteRolePolicy),
            "ListRolePolicies" => Some(Self::ListRolePolicies),
            "PutGroupPolicy" => Some(Self::PutGroupPolicy),
            "GetGroupPolicy" => Some(Self::GetGroupPolicy),
            "DeleteGroupPolicy" => Some(Self::DeleteGroupPolicy),
            "ListGroupPolicies" => Some(Self::ListGroupPolicies),
            "TagUser" => Some(Self::TagUser),
            "UntagUser" => Some(Self::UntagUser),
            "ListUserTags" => Some(Self::ListUserTags),
            "TagRole" => Some(Self::TagRole),
            "UntagRole" => Some(Self::UntagRole),
            "ListRoleTags" => Some(Self::ListRoleTags),
            "CreateServiceLinkedRole" => Some(Self::CreateServiceLinkedRole),
            "DeleteServiceLinkedRole" => Some(Self::DeleteServiceLinkedRole),
            "GetServiceLinkedRoleDeletionStatus" => Some(Self::GetServiceLinkedRoleDeletionStatus),
            "UpdateAssumeRolePolicy" => Some(Self::UpdateAssumeRolePolicy),
            "SimulatePrincipalPolicy" => Some(Self::SimulatePrincipalPolicy),
            "SimulateCustomPolicy" => Some(Self::SimulateCustomPolicy),
            "ListEntitiesForPolicy" => Some(Self::ListEntitiesForPolicy),
            "GetAccountAuthorizationDetails" => Some(Self::GetAccountAuthorizationDetails),
            "CreateOpenIDConnectProvider" => Some(Self::CreateOpenIDConnectProvider),
            "GetOpenIDConnectProvider" => Some(Self::GetOpenIDConnectProvider),
            "DeleteOpenIDConnectProvider" => Some(Self::DeleteOpenIDConnectProvider),
            "ListOpenIDConnectProviders" => Some(Self::ListOpenIDConnectProviders),
            "TagPolicy" => Some(Self::TagPolicy),
            "UntagPolicy" => Some(Self::UntagPolicy),
            "ListPolicyTags" => Some(Self::ListPolicyTags),
            "TagInstanceProfile" => Some(Self::TagInstanceProfile),
            "UntagInstanceProfile" => Some(Self::UntagInstanceProfile),
            "ListInstanceProfileTags" => Some(Self::ListInstanceProfileTags),
            _ => None,
        }
    }
}

impl std::fmt::Display for IamOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
