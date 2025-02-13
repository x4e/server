use crate::{
    channel::{self, Message},
    check_permission,
    hub::Hub,
    websocket::ServerMessage,
    Error, Result, ID,
};
use async_trait::async_trait;
use futures::stream::SplitSink;
use futures::SinkExt;
use pgp::Message as OpenPGPMessage;
use pgp::SignedSecretKey;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    convert::TryFrom,
    io::Read,
    sync::Arc,
};
use tantivy::{
    collector::TopDocs,
    directory::MmapDirectory,
    doc,
    query::QueryParser,
    schema::{Field, Schema, FAST, STORED, TEXT},
    Index, IndexReader, IndexWriter, LeasedItem, ReloadPolicy, Searcher,
};
use tokio::io::AsyncWriteExt;
use tokio::sync::{Mutex, RwLock};
use warp::ws::Message as WebSocketMessage;
use warp::ws::WebSocket;
use xactor::*;

use lazy_static::lazy_static;

pub mod client_command {
    use super::{message, Arc, Mutex, Result, SplitSink, WebSocket, WebSocketMessage, ID};

    /// Disconnects the client by unsubscribing them from everything (does not drop connection).
    #[message(result = "u128")]
    #[derive(Clone, Debug)]
    pub struct Connect {
        pub websocket_writer: Arc<Mutex<SplitSink<WebSocket, WebSocketMessage>>>,
    }
    /// Disconnects the client by unsubscribing them from everything (does not drop connection).
    #[message(result = "()")]
    #[derive(Clone, Debug)]
    pub struct Disconnect {
        pub connection_id: u128,
    }
    /// Subscribes the client to notifications on a hub (everything except for messages sent in channels in the hub).
    #[message(result = "Result")]
    #[derive(Clone, Debug)]
    pub struct SubscribeHub {
        pub user_id: String,
        pub hub_id: ID,
        pub connection_id: u128,
    }
    /// Unsubscribes the client from notifications in a hub, does not change channel subscriptions.
    #[message(result = "()")]
    #[derive(Debug, Clone)]
    pub struct UnsubscribeHub {
        pub hub_id: ID,
        pub connection_id: u128,
    }
    /// Subscribes the client to notifications of new messages in the given channel.
    #[message(result = "Result")]
    #[derive(Debug, Clone)]
    pub struct SubscribeChannel {
        pub user_id: String,
        pub hub_id: ID,
        pub channel_id: ID,
        pub connection_id: u128,
    }
    /// Unsubscribes the client to notifications of new messages in the given channel.
    #[message(result = "()")]
    #[derive(Debug, Clone)]
    pub struct UnsubscribeChannel {
        pub hub_id: ID,
        pub channel_id: ID,
        pub connection_id: u128,
    }
    /// Notifies other clients subscribed to the given channel that the given user has started typing.
    #[message(result = "Result")]
    #[derive(Debug, Clone)]
    pub struct StartTyping {
        pub user_id: String,
        pub hub_id: ID,
        pub channel_id: ID,
    }
    /// Notifies other clients subscribed to the given channel that the given user has stopped typing.
    #[message(result = "Result")]
    #[derive(Debug, Clone)]
    pub struct StopTyping {
        pub user_id: String,
        pub hub_id: ID,
        pub channel_id: ID,
    }
}

/// Fields for the Tantivy message schema.
#[derive(Clone)]
pub struct MessageSchemaFields {
    pub content: Field,
    pub id: Field,
}

/// Message to tell the message server that there is a new message in a channel.
#[message(result = "Result")]
#[derive(Clone, Debug)]
pub struct NewMessageForIndex {
    pub hub_id: ID,
    pub channel_id: ID,
    pub message: channel::Message,
}

/// Command for a [`MessageServer`] to search the given channel with a query.
#[message(result = "Result<Vec<ID>>")]
#[derive(Clone, Debug)]
pub struct SearchMessageIndex {
    /// ID of the hub the channel is in.
    pub hub_id: ID,
    /// ID of the channel in which to perform the search.
    pub channel_id: ID,
    /// Maximum number of results to return.
    pub limit: usize,
    /// Query string.
    pub query: String,
}

/// Types of updates that trigger [`ServerNotification::HubUpdated`]
#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum HubUpdateType {
    HubDeleted,
    HubRenamed,
    HubDescriptionUpdated,
    UserJoined(ID),
    UserLeft(ID),
    UserBanned(ID),
    UserMuted(ID),
    UserUnmuted(ID),
    UserUnbanned(ID),
    UserKicked(ID),
    UserHubPermissionChanged(ID),
    UserChannelPermissionChanged(ID, ID),
    UsernameChanged(ID),
    UserStatusUpdated(ID),
    UserDescriptionUpdated(ID),
    MemberNicknameChanged(ID),
    ChannelCreated(ID),
    ChannelDeleted(ID),
    ChannelRenamed(ID),
    ChannelDescriptionUpdated(ID),
}

/// Message to notify the server of a change made externally, usually used so the server can notify clients.
#[message(result = "()")]
#[derive(Debug, Clone)]
pub enum ServerNotification {
    NewMessage(ID, ID, ID, String, channel::Message),
    HubUpdated(ID, HubUpdateType),
}

/// Tells the [`Server`] to get an address to it's [`MessageServer`].
#[message(result = "Addr<MessageServer>")]
#[derive(Clone, Copy)]
pub struct GetMessageServer;

lazy_static! {
    static ref MESSAGE_SCHEMA: Schema = {
        let mut schema_builder = Schema::builder();
        schema_builder.add_text_field("content", TEXT);
        schema_builder.add_bytes_field("id", STORED | FAST);
        schema_builder.build()
    };
    static ref MESSAGE_SCHEMA_FIELDS: MessageSchemaFields = MessageSchemaFields {
        content: MESSAGE_SCHEMA
            .get_field("content")
            .expect("Failed to create a Tantivy schema correctly."),
        id: MESSAGE_SCHEMA
            .get_field("id")
            .expect("Failed to create a Tantivy schema correctly."),
    };
}

/// Adds a message to a Tantivy [`IndexWriter`].
pub fn add_message_to_writer(writer: &mut IndexWriter, message: channel::Message) -> Result {
    writer.add_document(doc!(
        MESSAGE_SCHEMA_FIELDS.id => bincode::serialize(&message.id)?,
        MESSAGE_SCHEMA_FIELDS.content => message.content,
    ));
    Ok(())
}

/// Logs the given message ID to a file, should be called after any Tantivy commits.
async fn log_last_message(hub_id: ID, channel_id: ID, message_id: ID) -> Result {
    let log_path_string = format!(
        "{}/{:x}/{:x}/log",
        crate::hub::HUB_DATA_FOLDER,
        hub_id.as_u128(),
        channel_id.as_u128()
    );
    tokio::fs::write(log_path_string, &message_id.as_u128().to_ne_bytes()).await?;
    Ok(())
}

async fn log_if_nologs(hub_id: ID, channel_id: ID, message_id: ID) -> Result {
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(format!(
            "{}/{:x}/{:x}/log",
            crate::hub::HUB_DATA_FOLDER,
            hub_id.as_u128(),
            channel_id.as_u128()
        ))
        .await?;
    file.write(&message_id.as_u128().to_ne_bytes()).await?;
    Ok(())
}

pub type IndexMap = HashMap<(ID, ID), Index>;
pub type IndexWriterMap = HashMap<(ID, ID), IndexWriter>;
pub type IndexReaderMap = HashMap<(ID, ID), IndexReader>;
pub type PendingMessageMap = HashMap<(ID, ID), (u8, ID)>;

pub struct MessageServer {
    indexes: IndexMap,
    index_writers: IndexWriterMap,
    index_readers: IndexReaderMap,
    pending_messages: PendingMessageMap,
}

impl MessageServer {
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
            index_writers: HashMap::new(),
            index_readers: HashMap::new(),
            pending_messages: HashMap::new(),
        }
    }

    /// Sets up the Tantivy index for a given channel, also makes sure that the index is up to date by commiting any messages sent after the last message sent (logged by [`log_last_message`]).
    async fn setup_index(&mut self, hub_id: ID, channel_id: ID) -> Result {
        let dir_string = format!(
            "{}/{:x}/{:x}/index",
            crate::hub::HUB_DATA_FOLDER,
            hub_id.as_u128(),
            channel_id.as_u128()
        );
        let dir_path = std::path::Path::new(&dir_string);
        if !dir_path.is_dir() {
            tokio::fs::create_dir_all(dir_path).await?;
        }
        let dir = MmapDirectory::open(dir_path)?;
        let index = Index::open_or_create(dir, MESSAGE_SCHEMA.clone())?;
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::OnCommit)
            .try_into()?;
        let mut writer = index.writer(50_000_000)?;
        let key = (hub_id, channel_id);
        let log_path_string = format!(
            "{}/{:x}/{:x}/log",
            crate::hub::HUB_DATA_FOLDER,
            hub_id.as_u128(),
            channel_id.as_u128()
        );
        let log_path = std::path::Path::new(&log_path_string);
        if log_path.is_file() {
            let mut buf: [u8; 16] = [0; 16];
            tokio::fs::read(log_path)
                .await?
                .as_slice()
                .read_exact(&mut buf)?;
            let last_id = ID::from_u128(u128::from_le_bytes(buf));
            let filename = format!("{}{:x}.json", crate::hub::HUB_INFO_FOLDER, hub_id.as_u128());
            let path = std::path::Path::new(&filename);
            if !path.exists() {
                return Err(Error::HubNotFound);
            }
            let json = tokio::fs::read_to_string(path).await?;
            let hub = serde_json::from_str::<Hub>(&json)?;
            if let Some(channel) = hub.channels.get(&channel_id) {
                let messages: Vec<Message> = channel
                    .get_all_messages_from(last_id)
                    .await
                    .iter()
                    .filter_map(|signed_message| Message::try_from(signed_message).ok())
                    .collect();
                let last_id = if let Some(last) = messages.last() {
                    Some(last.id)
                } else {
                    None
                };

                for message in messages {
                    add_message_to_writer(&mut writer, message)?;
                }
                writer.commit()?;
                if let Some(last_id) = last_id {
                    log_last_message(hub_id, channel_id, last_id).await?;
                }
                reader.reload()?;
            }
        }
        self.indexes.insert(key, index);
        self.index_readers.insert(key, reader);
        self.index_writers.insert(key, writer);
        Ok(())
    }

    /// Gets a reader for a Tantivy index, also runs [`setup_index`] if it hasn't already been run for the given channel.
    async fn get_reader(&mut self, hub_id: ID, channel_id: ID) -> Result<&IndexReader> {
        let key = (hub_id, channel_id);
        if !self.index_readers.contains_key(&key) {
            self.setup_index(hub_id, channel_id).await?;
        }
        if let Some(reader) = self.index_readers.get(&key) {
            Ok(reader)
        } else {
            Err(Error::GetIndexReader)
        }
    }

    /// Gets a searcher for the Tantivy index for a channel, uses [`get_reader`].
    async fn get_searcher(&mut self, hub_id: ID, channel_id: ID) -> Result<LeasedItem<Searcher>> {
        let reader = self.get_reader(hub_id, channel_id).await?;
        let _ = reader.reload();
        Ok(reader.searcher())
    }

    /// Gets a writer for a Tantivy index, also runs [`setup_index`] if it hasn't already been run for the given channel.
    async fn get_writer(&mut self, hub_id: ID, channel_id: ID) -> Result<&mut IndexWriter> {
        let key = (hub_id, channel_id);
        if !self.index_writers.contains_key(&key) {
            self.setup_index(hub_id, channel_id).await?;
        }
        if let Some(writer) = self.index_writers.get_mut(&key) {
            Ok(writer)
        } else {
            Err(Error::GetIndexWriter)
        }
    }
}

impl Default for MessageServer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Actor for MessageServer {
    async fn stopped(&mut self, _ctx: &mut xactor::Context<Self>) {
        for (hc_id, writer) in self.index_writers.iter_mut() {
            if let Some((_, id)) = self.pending_messages.get(&hc_id) {
                let _ = log_last_message(hc_id.0, hc_id.1, *id);
            }
            let _ = writer.commit();
        }
    }
}

#[async_trait]
impl Handler<SearchMessageIndex> for MessageServer {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        msg: SearchMessageIndex,
    ) -> Result<Vec<ID>> {
        {
            let pending = {
                self.pending_messages
                    .get(&(msg.hub_id, msg.channel_id))
                    .cloned()
            };
            if let Some(pending) = pending {
                if pending.0 != 0 {
                    let _ = self.get_writer(msg.hub_id, msg.channel_id).await?.commit();
                    log_last_message(msg.hub_id, msg.channel_id, pending.1).await?;

                    self.pending_messages
                        .insert((msg.hub_id, msg.channel_id), (0, pending.1));
                }
            }
        }
        let searcher = self.get_searcher(msg.hub_id, msg.channel_id).await?;
        let query_parser =
            QueryParser::for_index(searcher.index(), vec![MESSAGE_SCHEMA_FIELDS.content]);
        let query = query_parser.parse_query(&msg.query)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(msg.limit))?;
        let mut result = Vec::new();
        for (_score, doc_address) in top_docs {
            let retrieved_doc = searcher.doc(doc_address)?;
            if let Some(value) = retrieved_doc.get_first(MESSAGE_SCHEMA_FIELDS.id) {
                if let Some(bytes) = value.bytes_value() {
                    if let Ok(id) = bincode::deserialize::<ID>(bytes) {
                        result.push(id);
                    }
                }
            }
        }
        Ok(result)
    }
}

#[async_trait]
impl Handler<NewMessageForIndex> for MessageServer {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: NewMessageForIndex) -> Result {
        let mut new_pending: u8;
        let message_id = msg.message.id;
        if let Some((pending, _)) = self
            .pending_messages
            .get(&(msg.hub_id, msg.channel_id))
            .cloned()
        {
            new_pending = pending + 1;
            if pending >= crate::TANTIVY_COMMIT_THRESHOLD {
                let mut writer = self.get_writer(msg.hub_id, msg.channel_id).await?;
                add_message_to_writer(&mut writer, msg.message)?;
                writer.commit()?;
                log_last_message(msg.hub_id, msg.channel_id, message_id).await?;
                new_pending = 0;
            } else {
                log_if_nologs(msg.hub_id, msg.channel_id, message_id).await?;
            }
        } else {
            new_pending = 1;
            log_if_nologs(msg.hub_id, msg.channel_id, message_id).await?;
        }
        let _ = self
            .pending_messages
            .insert((msg.hub_id, msg.channel_id), (new_pending, message_id));
        Ok(())
    }
}

pub type SubscribedChannelMap = Arc<RwLock<HashMap<(ID, ID), Arc<RwLock<HashSet<u128>>>>>>;
pub type SubscribedHubMap = Arc<RwLock<HashMap<ID, Arc<RwLock<HashSet<u128>>>>>>;
pub type SubscribedMap = Arc<RwLock<HashMap<u128, Arc<RwLock<(HashSet<(ID, ID)>, HashSet<ID>)>>>>>;
pub type ConnectedMap =
    Arc<RwLock<HashMap<u128, Arc<Mutex<SplitSink<WebSocket, WebSocketMessage>>>>>>;

/// Server that handles socket clients and manages notifying them of new messages/changes as well as sending messages to be indexed by Tantivy.
pub struct Server {
    subscribed_channels: SubscribedChannelMap,
    subscribed_hubs: SubscribedHubMap,
    subscribed: SubscribedMap,
    connected: ConnectedMap,
    message_server: Addr<MessageServer>,
    secret_key: SignedSecretKey,
}

impl Server {
    /// Creates a new server with default options, also creates a [`MessageServer`] with the given `commit_threshold` (how many messages should be added to the search index before commiting to the index).
    pub async fn new(secret_key: SignedSecretKey) -> Result<Self> {
        Ok(Self {
            subscribed_channels: Arc::new(RwLock::new(HashMap::new())),
            subscribed_hubs: Arc::new(RwLock::new(HashMap::new())),
            subscribed: Arc::new(RwLock::new(HashMap::new())),
            connected: Arc::new(RwLock::new(HashMap::new())),
            secret_key,
            message_server: MessageServer::new()
                .start()
                .await
                .map_err(|_| Error::ServerStartFailed)?,
        })
    }

    /// Sends a [`ServreMessage`] to all clients subscribed to notifications for the given hub.
    async fn send_hub(&self, message: ServerMessage, hub_id: &ID) -> Result {
        if let Some(subscribed_arc) = self.subscribed_hubs.read().await.get(hub_id) {
            let signed_message =
                OpenPGPMessage::new_literal("", serde_json::to_string(&message)?.as_str()).sign(
                    &self.secret_key,
                    String::new,
                    pgp::crypto::HashAlgorithm::SHA2_256,
                )?;
            let signed_message_string = signed_message.to_armored_string(None)?;
            let message = WebSocketMessage::text(signed_message_string);
            for connection_id in subscribed_arc.read().await.iter() {
                if let Some(connection) = self.connected.read().await.get(connection_id) {
                    let _ = connection.lock().await.send(message.clone()).await;
                }
            }
        }
        Ok(())
    }

    /// Sends a [`ServreMessage`] to all clients subscribed to notifications for the given channel.
    async fn send_channel(&self, message: ServerMessage, hub_id: ID, channel_id: ID) -> Result {
        if let Some(subscribed_arc) = self
            .subscribed_channels
            .read()
            .await
            .get(&(hub_id, channel_id))
        {
            let signed_message =
                OpenPGPMessage::new_literal("", serde_json::to_string(&message)?.as_str()).sign(
                    &self.secret_key,
                    String::new,
                    pgp::crypto::HashAlgorithm::SHA2_256,
                )?;
            let signed_message_string = signed_message.to_armored_string(None)?;
            let message = WebSocketMessage::text(signed_message_string);
            for connection_id in subscribed_arc.read().await.iter() {
                if let Some(connection) = self.connected.read().await.get(connection_id) {
                    let _ = connection.lock().await.send(message.clone()).await;
                }
            }
        }
        Ok(())
    }
}

impl Actor for Server {}

#[async_trait]
impl Handler<client_command::Connect> for Server {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: client_command::Connect) -> u128 {
        let mut connection_set = self.connected.write().await;
        let mut id = rand::random::<u128>();
        while connection_set.contains_key(&id) {
            id = rand::random::<u128>();
        }
        connection_set.insert(id, msg.websocket_writer);
        id
    }
}

#[async_trait]
impl Handler<client_command::Disconnect> for Server {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: client_command::Disconnect) {
        if let Some(subscribed) = self.subscribed.write().await.remove(&msg.connection_id) {
            let subscribed = subscribed.write().await;
            let subscribed_channels = self.subscribed_channels.write().await;
            for channel in subscribed.0.iter() {
                if let Some(subs) = subscribed_channels.get(&channel) {
                    subs.write().await.remove(&msg.connection_id);
                }
            }
            drop(subscribed_channels);
            let subscribed_hubs = self.subscribed_hubs.write().await;
            for hub in subscribed.1.iter() {
                if let Some(subs) = subscribed_hubs.get(&hub) {
                    subs.write().await.remove(&msg.connection_id);
                }
            }
            drop(subscribed_hubs);
            self.connected.write().await.remove(&msg.connection_id);
        }
    }
}

#[async_trait]
impl Handler<client_command::SubscribeHub> for Server {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        msg: client_command::SubscribeHub,
    ) -> Result {
        Hub::load(msg.hub_id)
            .await
            .and_then(|hub| Ok(hub.get_member(&msg.user_id)?.clone()))?;
        self.subscribed
            .write()
            .await
            .entry(msg.connection_id)
            .or_default()
            .write()
            .await
            .1
            .insert(msg.hub_id);
        self.subscribed_hubs
            .write()
            .await
            .entry(msg.hub_id)
            .or_default()
            .write()
            .await
            .insert(msg.connection_id);
        Ok(())
    }
}

#[async_trait]
impl Handler<client_command::UnsubscribeHub> for Server {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: client_command::UnsubscribeHub) {
        if let Some(subs) = self.subscribed.write().await.get(&msg.connection_id) {
            subs.write().await.1.remove(&msg.hub_id);
        }
        if let Some(subs) = self.subscribed_hubs.write().await.get(&msg.hub_id) {
            subs.write().await.remove(&msg.connection_id);
        }
    }
}

#[async_trait]
impl Handler<client_command::SubscribeChannel> for Server {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        msg: client_command::SubscribeChannel,
    ) -> Result {
        Hub::load(msg.hub_id)
            .await
            .and_then(|hub| {
                if let Ok(member) = hub.get_member(&msg.user_id) {
                    let member = member.clone();
                    Ok((hub, member))
                } else {
                    Err(Error::MemberNotFound)
                }
            })
            .and_then(|(hub, user)| {
                check_permission!(
                    user,
                    msg.channel_id,
                    crate::permission::ChannelPermission::Read,
                    hub
                );
                Ok(())
            })?;
        let key = (msg.hub_id, msg.channel_id);
        self.subscribed
            .write()
            .await
            .entry(msg.connection_id)
            .or_default()
            .write()
            .await
            .0
            .insert(key);
        self.subscribed_channels
            .write()
            .await
            .entry(key)
            .or_default()
            .write()
            .await
            .insert(msg.connection_id);
        Ok(())
    }
}

#[async_trait]
impl Handler<client_command::UnsubscribeChannel> for Server {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: client_command::UnsubscribeChannel) {
        let key = (msg.hub_id, msg.channel_id);
        if let Some(subs) = self.subscribed.write().await.get(&msg.connection_id) {
            subs.write().await.0.remove(&key);
        }
        if let Some(subs) = self.subscribed_channels.write().await.get(&key) {
            subs.write().await.remove(&msg.connection_id);
        }
    }
}

#[async_trait]
impl Handler<client_command::StartTyping> for Server {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        msg: client_command::StartTyping,
    ) -> Result {
        Hub::load(msg.hub_id)
            .await
            .and_then(|hub| {
                if let Ok(member) = hub.get_member(&msg.user_id) {
                    let member = member.clone();
                    Ok((hub, member))
                } else {
                    Err(Error::MemberNotFound)
                }
            })
            .and_then(|(hub, user)| {
                check_permission!(
                    user,
                    msg.channel_id,
                    crate::permission::ChannelPermission::Write,
                    hub
                );
                Ok(())
            })?;
        let _ = self
            .send_channel(
                ServerMessage::UserStartedTyping {
                    user_id: msg.user_id,
                    hub_id: msg.hub_id,
                    channel_id: msg.channel_id,
                },
                msg.hub_id,
                msg.channel_id,
            )
            .await;
        Ok(())
    }
}

#[async_trait]
impl Handler<client_command::StopTyping> for Server {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        msg: client_command::StopTyping,
    ) -> Result {
        Hub::load(msg.hub_id)
            .await
            .and_then(|hub| {
                if let Ok(member) = hub.get_member(&msg.user_id) {
                    let member = member.clone();
                    Ok((hub, member))
                } else {
                    Err(Error::MemberNotFound)
                }
            })
            .and_then(|(hub, user)| {
                check_permission!(
                    user,
                    msg.channel_id,
                    crate::permission::ChannelPermission::Write,
                    hub
                );
                Ok(())
            })?;
        let _ = self
            .send_channel(
                ServerMessage::UserStoppedTyping {
                    user_id: msg.user_id,
                    hub_id: msg.hub_id,
                    channel_id: msg.channel_id,
                },
                msg.hub_id,
                msg.channel_id,
            )
            .await;
        Ok(())
    }
}

#[async_trait]
impl Handler<ServerNotification> for Server {
    async fn handle(&mut self, _ctx: &mut Context<Self>, msg: ServerNotification) {
        match msg {
            ServerNotification::NewMessage(
                hub_id,
                channel_id,
                message_id,
                armoured_message,
                message,
            ) => {
                let _ = self
                    .message_server
                    .call(NewMessageForIndex {
                        hub_id,
                        channel_id,
                        message,
                    })
                    .await;
                let _ = self
                    .send_channel(
                        ServerMessage::ChatMessage {
                            hub_id,
                            channel_id,
                            message_id,
                            armoured_message,
                        },
                        hub_id,
                        channel_id,
                    )
                    .await;
            }
            ServerNotification::HubUpdated(hub_id, update_type) => {
                let _ = self
                    .send_hub(
                        ServerMessage::HubUpdated {
                            hub_id,
                            update_type,
                        },
                        &hub_id,
                    )
                    .await;
            }
        }
    }
}

#[async_trait]
impl Handler<GetMessageServer> for Server {
    async fn handle(
        &mut self,
        _ctx: &mut Context<Self>,
        _msg: GetMessageServer,
    ) -> Addr<MessageServer> {
        self.message_server.clone()
    }
}
