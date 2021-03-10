use std::collections::{HashMap, HashSet};

use rayon::iter::{IndexedParallelIterator, IntoParallelRefIterator};
use serde::{Deserialize, Serialize};

use crate::{
    channel::{Channel, Message},
    get_system_millis, is_valid_username, new_id,
    permission::{
        ChannelPermission, ChannelPermissions, HubPermission, HubPermissions, PermissionSetting,
    },
    user::User,
    ApiActionError, JsonLoadError, JsonSaveError, ID,
};

pub static HUB_DATA_FOLDER: &str = "data/hubs/info";

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct HubMember {
    pub user: ID,
    pub joined: u128,
    pub hub: ID,
    pub nickname: String,
    pub groups: Vec<ID>,
    pub hub_permissions: HubPermissions,
    pub channel_permissions: HashMap<ID, ChannelPermissions>,
}

impl HubMember {
    pub fn new(user: &User, hub: ID) -> Self {
        Self {
            nickname: user.username.clone(),
            user: user.id.clone(),
            hub,
            groups: Vec::new(),
            joined: get_system_millis(),
            hub_permissions: HashMap::new(),
            channel_permissions: HashMap::new(),
        }
    }

    pub fn set_nickname(&mut self, nickname: String) -> Result<(), ()> {
        if is_valid_username(&nickname) {
            self.nickname = nickname;
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn join_group(&mut self, group: &mut PermissionGroup) {
        if !self.groups.contains(&group.id) {
            self.groups.push(group.id.clone());
        }
        if !group.members.contains(&self.user) {
            group.members.push(self.user.clone());
        }
    }

    pub fn leave_group(&mut self, group: &mut PermissionGroup) {
        if let Some(index) = self.groups.par_iter().position_any(|id| id == &group.id) {
            self.groups.remove(index);
        }
        if let Some(index) = group.members.par_iter().position_any(|id| id == &self.user) {
            group.members.remove(index);
        }
    }

    pub fn set_permission(&mut self, permission: HubPermission, value: PermissionSetting) {
        self.hub_permissions.insert(permission, value);
    }

    pub fn set_channel_permission(
        &mut self,
        channel: ID,
        permission: ChannelPermission,
        value: PermissionSetting,
    ) {
        let channel_permissions = self
            .channel_permissions
            .entry(channel)
            .or_insert(HashMap::new());
        channel_permissions.insert(permission, value);
    }

    pub fn has_all_permissions(&self) -> bool {
        if let Some(value) = self.hub_permissions.get(&HubPermission::All) {
            if value == &PermissionSetting::TRUE {
                return true;
            }
        }
        false
    }

    pub fn has_permission(&self, permission: HubPermission, hub: &Hub) -> bool {
        println!("{:?}", self.hub_permissions);
        if hub.owner == self.user {
            return true;
        }
        if self.has_all_permissions() {
            return true;
        }
        println!("passed all");
        if let Some(value) = self.hub_permissions.get(&permission) {
            match value {
                &PermissionSetting::TRUE => {
                    return true;
                }
                &PermissionSetting::FALSE => {
                    return false;
                }
                PermissionSetting::NONE => {}
            };
        } else {
            for group in self.groups.iter() {
                if let Some(group) = hub.groups.get(&group) {
                    if group.has_permission(&permission) {
                        return true;
                    }
                }
            }
        }
        false
    }

    pub fn has_channel_permission(
        &self,
        channel: &ID,
        permission: &ChannelPermission,
        hub: &Hub,
    ) -> bool {
        if hub.owner == self.user {
            return true;
        }
        if self.has_all_permissions() {
            return true;
        }
        if let Some(channel) = self.channel_permissions.get(channel) {
            if let Some(value) = channel.get(&ChannelPermission::All) {
                if value == &PermissionSetting::TRUE {
                    return true;
                }
            }
            if let Some(value) = channel.get(permission) {
                match value {
                    &PermissionSetting::TRUE => {
                        return true;
                    }
                    &PermissionSetting::FALSE => {
                        return false;
                    }
                    PermissionSetting::NONE => {
                        if self.has_permission(permission.hub_equivalent(), hub) {
                            return true;
                        }
                    }
                };
            }
        } else {
            for group in self.groups.iter() {
                if let Some(group) = hub.groups.get(&group) {
                    if group.has_channel_permission(channel, permission) {
                        return true;
                    }
                }
            }
        }
        false
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct PermissionGroup {
    pub id: ID,
    pub name: String,
    pub members: Vec<ID>,
    pub hub_permissions: HubPermissions,
    pub channel_permissions: HashMap<ID, ChannelPermissions>,
    pub created: u128,
}

impl PermissionGroup {
    pub fn new(name: String, id: ID) -> Self {
        Self {
            created: get_system_millis(),
            id,
            name,
            members: Vec::new(),
            hub_permissions: HashMap::new(),
            channel_permissions: HashMap::new(),
        }
    }

    pub fn add_member(&mut self, user: &mut HubMember) {
        user.join_group(self)
    }

    pub fn remove_member(&mut self, user: &mut HubMember) {
        user.leave_group(self)
    }

    pub fn set_permission(&mut self, permission: HubPermission, value: PermissionSetting) {
        self.hub_permissions.insert(permission, value);
    }

    pub fn set_channel_permission(
        &mut self,
        channel: ID,
        permission: ChannelPermission,
        value: PermissionSetting,
    ) {
        let channel_permissions = self
            .channel_permissions
            .entry(channel)
            .or_insert(HashMap::new());
        channel_permissions.insert(permission, value);
    }

    pub fn has_all_permissions(&self) -> bool {
        if let Some(value) = self.hub_permissions.get(&HubPermission::All) {
            if value == &PermissionSetting::TRUE {
                return true;
            }
        }
        false
    }

    pub fn has_permission(&self, permission: &HubPermission) -> bool {
        if self.has_all_permissions() {
            return true;
        }
        if let Some(value) = self.hub_permissions.get(permission) {
            if value == &PermissionSetting::TRUE {
                return true;
            }
        }
        return false;
    }

    pub fn has_channel_permission(&self, channel: &ID, permission: &ChannelPermission) -> bool {
        if self.has_all_permissions() {
            return true;
        }
        if let Some(channel) = self.channel_permissions.get(channel) {
            if let Some(value) = channel.get(&ChannelPermission::All) {
                if value == &PermissionSetting::TRUE {
                    return true;
                }
            }
            if let Some(value) = channel.get(&permission) {
                if value == &PermissionSetting::TRUE {
                    return true;
                } else if value == &PermissionSetting::NONE {
                    if self.has_permission(&permission.hub_equivalent()) {
                        return true;
                    }
                }
            }
        }
        return false;
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Hub {
    pub channels: HashMap<ID, Channel>,
    pub members: HashMap<ID, HubMember>,
    pub bans: HashSet<ID>,
    pub mutes: HashSet<ID>,
    pub owner: ID,
    pub groups: HashMap<ID, PermissionGroup>,
    pub default_group: ID,
    pub name: String,
    pub id: ID,
    pub created: u128,
}

impl Hub {
    pub fn new(name: String, id: ID, creator: &User) -> Self {
        let creator_id = creator.id.clone();
        let mut everyone = PermissionGroup::new(String::from("everyone"), new_id());
        let mut owner = HubMember::new(creator, id.clone());
        let mut members = HashMap::new();
        let mut groups = HashMap::new();
        owner.join_group(&mut everyone);
        owner.set_permission(HubPermission::All, PermissionSetting::TRUE);
        members.insert(creator_id.clone(), owner);
        groups.insert(everyone.id.clone(), everyone.clone());
        Self {
            name,
            id,
            groups,
            default_group: everyone.id.clone(),
            owner: creator_id,
            bans: HashSet::new(),
            mutes: HashSet::new(),
            channels: HashMap::new(),
            members,
            created: get_system_millis(),
        }
    }

    pub async fn new_channel(&mut self, member_id: ID, name: String) -> Result<ID, ApiActionError> {
        if is_valid_username(&name) {
            if let Some(member) = self.members.get(&member_id) {
                if member
                    .clone()
                    .has_permission(HubPermission::CreateChannel, self)
                {
                    let mut id = new_id();
                    while self.channels.contains_key(&id) {
                        id = new_id();
                    }
                    if let Ok(channel) = Channel::new(name, id.clone(), self.id.clone()).await {
                        self.channels.insert(id.clone(), channel);
                        if let Ok(_) = self.save().await {
                            Ok(id)
                        } else {
                            Err(ApiActionError::WriteFileError)
                        }
                    } else {
                        Err(ApiActionError::WriteFileError)
                    }
                } else {
                    Err(ApiActionError::NoPermission)
                }
            } else {
                Err(ApiActionError::NotInHub)
            }
        } else {
            Err(ApiActionError::BadNameCharacters)
        }
    }

    pub async fn rename_channel(
        &mut self,
        user: ID,
        channel: ID,
        name: String,
    ) -> Result<String, ApiActionError> {
        if is_valid_username(&name) {
            if let Some(user) = self.members.get(&user) {
                if user.clone().has_channel_permission(
                    &channel,
                    &ChannelPermission::ViewChannel,
                    self,
                ) {
                    if let Some(channel) = self.channels.get_mut(&channel) {
                        let old_name = channel.name.clone();
                        channel.name = name;
                        if let Ok(_) = self.save().await {
                            Ok(old_name)
                        } else {
                            Err(ApiActionError::WriteFileError)
                        }
                    } else {
                        Err(ApiActionError::ChannelNotFound)
                    }
                } else {
                    Err(ApiActionError::NoPermission)
                }
            } else {
                Err(ApiActionError::NotInHub)
            }
        } else {
            Err(ApiActionError::BadNameCharacters)
        }
    }

    pub async fn delete_channel(&mut self, user: ID, channel: ID) -> Result<(), ApiActionError> {
        if let Some(user) = self.members.get(&user) {
            if user
                .clone()
                .has_permission(HubPermission::DeleteChannel, self)
            {
                if user.clone().has_channel_permission(
                    &channel,
                    &ChannelPermission::ViewChannel,
                    self,
                ) {
                    if let Some(_) = self.channels.remove(&channel) {
                        if let Ok(_) = self.save().await {
                            Ok(())
                        } else {
                            Err(ApiActionError::WriteFileError)
                        }
                    } else {
                        Err(ApiActionError::ChannelNotFound)
                    }
                } else {
                    Err(ApiActionError::NoPermission)
                }
            } else {
                Err(ApiActionError::NoPermission)
            }
        } else {
            Err(ApiActionError::NotInHub)
        }
    }

    pub async fn send_message(
        &mut self,
        user: ID,
        channel: ID,
        message: String,
    ) -> Result<ID, ApiActionError> {
        if let Some(member) = self.members.get(&user) {
            if !self.mutes.contains(&user) {
                if member.clone().has_channel_permission(
                    &channel,
                    &ChannelPermission::SendMessage,
                    self,
                ) {
                    if let Some(channel) = self.channels.get_mut(&channel) {
                        let id = new_id();
                        let message = Message {
                            id: id.clone(),
                            sender: member.user.clone(),
                            created: get_system_millis(),
                            content: message,
                        };
                        channel.add_message(message).await?;
                        Ok(id)
                    } else {
                        Err(ApiActionError::ChannelNotFound)
                    }
                } else {
                    Err(ApiActionError::NoPermission)
                }
            } else {
                Err(ApiActionError::Muted)
            }
        } else {
            Err(ApiActionError::NotInHub)
        }
    }

    pub async fn save(&self) -> Result<(), JsonSaveError> {
        if let Err(_) = tokio::fs::create_dir_all(HUB_DATA_FOLDER).await {
            return Err(JsonSaveError::Directory);
        }
        if let Ok(json) = serde_json::to_string(self) {
            if let Ok(result) = tokio::fs::write(
                HUB_DATA_FOLDER.to_owned() + "/" + &self.id.to_string() + ".json",
                json,
            )
            .await
            {
                Ok(result)
            } else {
                Err(JsonSaveError::WriteFile)
            }
        } else {
            Err(JsonSaveError::Serialize)
        }
    }

    pub async fn load(id: ID) -> Result<Self, JsonLoadError> {
        if let Ok(json) = tokio::fs::read_to_string(
            HUB_DATA_FOLDER.to_owned() + "/" + &id.to_string() + ".json",
        )
        .await
        {
            if let Ok(result) = serde_json::from_str(&json) {
                Ok(result)
            } else {
                Err(JsonLoadError::Deserialize)
            }
        } else {
            Err(JsonLoadError::ReadFile)
        }
    }

    pub fn user_join(&mut self, user: &User) -> Result<HubMember, ()> {
        let mut member = HubMember::new(user, self.id.clone());
        if let Some(group) = self.groups.get_mut(&self.default_group) {
            group.add_member(&mut member);
            self.members.insert(member.user.clone(), member.clone());
            Ok(member)
        } else {
            Err(())
        }
    }

    pub fn user_leave(&mut self, user: &User) -> Result<(), ()> {
        let mut member = HubMember::new(user, self.id.clone());
        if let Some(group) = self.groups.get_mut(&self.default_group) {
            member.leave_group(group);
            self.members.remove(&user.id);
            Ok(())
        } else {
            Err(())
        }
    }

    pub fn channels(&mut self, user: ID) -> Result<Vec<Channel>, ApiActionError> {
        let hub_im = self.clone();
        if let Some(user) = self.members.get_mut(&user) {
            let mut result = Vec::new();
            for channel in self.channels.clone() {
                if user.has_channel_permission(&channel.0, &ChannelPermission::ViewChannel, &hub_im)
                {
                    result.push(channel.1.clone());
                }
            }
            Ok(result)
        } else {
            Err(ApiActionError::UserNotFound)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        auth::Service,
        permission::{ChannelPermission, HubPermission, PermissionSetting},
        user::User,
        ID,
    };

    use super::{Hub, HubMember, PermissionGroup};

    fn get_user_for_test(id: u128) -> User {
        User::new(
            ID::from_u128(id).to_string(),
            "test_user@example.com".to_string(),
            Service::GitHub,
        )
    }

    fn get_hub_for_test() -> Hub {
        Hub::new("test".to_string(), ID::from_u128(1), &get_user_for_test(1))
    }

    #[test]
    fn hub_creator_permissions() {
        let member = HubMember::new(&get_user_for_test(1), ID::from_u128(1));
        let hub = get_hub_for_test();
        assert!(member.has_permission(HubPermission::All, &hub));
    }

    #[test]
    fn hub_permissions() {
        let mut hub = get_hub_for_test();
        let mut member = hub
            .user_join(&get_user_for_test(2))
            .expect("Test user could not join test hub.");
        assert!(!member.has_permission(HubPermission::All, &hub));
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        member.set_permission(HubPermission::SendMessage, PermissionSetting::FALSE);
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        member.set_permission(HubPermission::SendMessage, PermissionSetting::NONE);
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        member.set_permission(HubPermission::SendMessage, PermissionSetting::TRUE);
        assert!(member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        member.set_permission(HubPermission::All, PermissionSetting::TRUE);
        assert!(member.has_permission(HubPermission::ReadMessage, &hub));
        assert!(member.has_permission(HubPermission::SendMessage, &hub));
    }

    #[test]
    fn channel_permissions() {
        let mut hub = get_hub_for_test();
        let mut member = hub
            .user_join(&get_user_for_test(2))
            .expect("Test user could not join test hub.");
        assert!(!member.has_permission(HubPermission::All, &hub));
        let id = ID::from_u128(0);
        assert!(!member.has_channel_permission(&id, &ChannelPermission::SendMessage, &hub));
        assert!(!member.has_channel_permission(&id, &ChannelPermission::ReadMessage, &hub));
        member.set_channel_permission(
            id.clone(),
            ChannelPermission::SendMessage,
            PermissionSetting::FALSE,
        );
        assert!(!member.has_channel_permission(&id, &ChannelPermission::SendMessage, &hub));
        assert!(!member.has_channel_permission(&id, &ChannelPermission::ReadMessage, &hub));
        member.set_channel_permission(
            id.clone(),
            ChannelPermission::SendMessage,
            PermissionSetting::NONE,
        );
        assert!(!member.has_channel_permission(&id, &ChannelPermission::SendMessage, &hub));
        assert!(!member.has_channel_permission(&id, &ChannelPermission::ReadMessage, &hub));
        member.set_channel_permission(
            id.clone(),
            ChannelPermission::SendMessage,
            PermissionSetting::TRUE,
        );
        assert!(member.has_channel_permission(&id, &ChannelPermission::SendMessage, &hub));
        assert!(!member.has_channel_permission(&id, &ChannelPermission::ReadMessage, &hub));
        member.set_permission(HubPermission::All, PermissionSetting::TRUE);
        assert!(member.has_channel_permission(&id, &ChannelPermission::SendMessage, &hub));
        assert!(member.has_channel_permission(&id, &ChannelPermission::ReadMessage, &hub));
    }

    #[test]
    fn group_permissions() {
        let mut hub = get_hub_for_test();
        let mut member = hub
            .user_join(&get_user_for_test(2))
            .expect("Test user could not join test hub.");
        let group = PermissionGroup::new("test_group".to_string(), ID::from_u128(0));
        hub.groups.insert(group.id.clone(), group.clone());
        member.join_group(
            hub.groups
                .get_mut(&group.id)
                .expect("Failed to get test group."),
        );
        assert!(!member.has_permission(HubPermission::All, &hub));
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        hub.groups
            .get_mut(&group.id)
            .expect("Failed to get test group.")
            .set_permission(HubPermission::SendMessage, PermissionSetting::FALSE);
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        hub.groups
            .get_mut(&group.id)
            .expect("Failed to get test group.")
            .set_permission(HubPermission::SendMessage, PermissionSetting::NONE);
        assert!(!member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        hub.groups
            .get_mut(&group.id)
            .expect("Failed to get test group.")
            .set_permission(HubPermission::SendMessage, PermissionSetting::TRUE);
        assert!(member.has_permission(HubPermission::SendMessage, &hub));
        assert!(!member.has_permission(HubPermission::ReadMessage, &hub));
        hub.groups
            .get_mut(&group.id)
            .expect("Failed to get test group.")
            .set_permission(HubPermission::All, PermissionSetting::TRUE);
        assert!(member.has_permission(HubPermission::ReadMessage, &hub));
        assert!(member.has_permission(HubPermission::SendMessage, &hub));
    }

    #[tokio::test]
    async fn channel_view() {
        let mut hub = get_hub_for_test();
        let mut member = hub
            .user_join(&get_user_for_test(2))
            .expect("Test user could not join test hub.");
        {
            {
                let member_in_hub = hub
                    .members
                    .get_mut(&member.user)
                    .expect("Failed to get hub member.");
                member_in_hub.set_permission(HubPermission::CreateChannel, PermissionSetting::TRUE);
                member = member_in_hub.clone();
            }
            assert!(!member.has_permission(HubPermission::All, &hub));
            assert!(member.has_permission(HubPermission::CreateChannel, &hub));
        }
        let channel_0 = hub
            .new_channel(member.user.clone(), "test0".to_string())
            .await
            .expect("Failed to create test channel.");
        let _channel_1 = hub
            .new_channel(member.user.clone(), "test1".to_string())
            .await
            .expect("Failed to create test channel.");
        assert!(hub
            .channels(member.user.clone())
            .expect("Failed to get hub channels.")
            .is_empty());

        {
            let member_in_hub = hub
                .members
                .get_mut(&member.user)
                .expect("Failed to get hub member.");
            member_in_hub.set_channel_permission(
                channel_0.clone(),
                ChannelPermission::ViewChannel,
                PermissionSetting::TRUE,
            );
        }
        let get = hub
            .channels(member.user.clone())
            .expect("Failed to get hub channels.");
        assert_eq!(get.len(), 1);
        assert_eq!(get[0].id, channel_0.clone());
    }

    #[tokio::test]
    async fn save_load() {
        let hub = Hub::new(
            "test".to_string(),
            ID::from_u128(1234),
            &get_user_for_test(1),
        );
        let _remove = tokio::fs::remove_file(
            "data/hubs/info/".to_string() + &ID::from_u128(1234).to_string() + ".json",
        )
        .await;
        hub.save().await.expect("Failed to save hub info.");
        let load = Hub::load(hub.id).await.expect("Failed to load hub info.");
        assert_eq!(hub, load);
    }
}
