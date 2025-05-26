use core::sync::atomic::AtomicU32;
use core::sync::atomic::Ordering;
use std::collections::HashMap;

use crossbeam_channel::Sender;
use serde::Deserialize;
use serde::Serialize;
use serde_repr::Deserialize_repr;
use serde_repr::Serialize_repr;
use tracing::error;
use zbus::interface;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::Type;
use zbus::zvariant::Value;

use crate::channel_utils::DiscardingSender;
use crate::serialization::ForwardedNotification;
use crate::serialization::NotificationRequests;
use crate::serialization::Request;
use crate::serialization::SendType;

#[derive(Debug, Type, Serialize, Deserialize, Clone)]
pub struct ServerInformation {
    /// The product name of the server.
    name: String,

    /// The vendor name. For example "KDE," "GNOME," "freedesktop.org" or "Microsoft".
    vendor: String,

    /// The server's version number.
    version: String,

    /// The specification version the server is compliant with.
    spec_version: String,
}

impl Default for ServerInformation {
    fn default() -> Self {
        Self {
            name: String::from(env!("CARGO_CRATE_NAME")),
            vendor: String::from(env!("CARGO_PKG_AUTHORS")),
            version: String::from(env!("CARGO_PKG_VERSION")),
            spec_version: String::from("1.3"),
        }
    }
}

pub struct Notifications {
    server_information: ServerInformation,
    current_id: AtomicU32,
    sender: DiscardingSender<Sender<SendType<Request>>>,
}

impl Notifications {
    pub fn new(sender: DiscardingSender<Sender<SendType<Request>>>) -> Self {
        Self {
            sender,
            server_information: ServerInformation::default(),
            current_id: AtomicU32::default(),
        }
    }
}

#[derive(Serialize_repr, Deserialize_repr, Debug, Clone, Copy, Type)]
#[repr(u8)]
pub(super) enum CloseReason {
    Expired = 1,
    UserDismissed = 2,
    CallClose = 3,
    Undefined = 4,
}

#[derive(Debug)]
pub enum NotificationSignals {
    Close(u32),
    Action(u32, String),
}

impl TryFrom<NotificationClosed> for NotificationSignals {
    type Error = zbus::Error;

    fn try_from(value: NotificationClosed) -> std::result::Result<Self, Self::Error> {
        let args = value.args()?;

        Ok(Self::Close(args.id))
    }
}

impl TryFrom<ActionInvoked> for NotificationSignals {
    type Error = zbus::Error;

    fn try_from(value: ActionInvoked) -> std::result::Result<Self, Self::Error> {
        let args = value.args()?;

        Ok(Self::Action(args.id, args.action_key.to_string()))
    }
}

#[interface(
    name = "org.freedesktop.Notifications",
    proxy(
        gen_blocking = false,
        default_path = "/org/freedesktop/Notifications",
        default_service = "org.freedesktop.Notifications",
    )
)]
impl Notifications {
    /// CloseNotification method
    pub(super) async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) {
        let Err(err) = self.sender.send(SendType::Object(Request::Notification(
            NotificationRequests::Close(id),
        ))) else {
            let Err(err) = emitter
                .notification_closed(id, CloseReason::CallClose)
                .await
            else {
                return;
            };

            error!("failed to emit notification close signal {:?}", err);
            return;
        };

        error!(
            "failed to send close notification request to client {:?}",
            err
        );
    }

    pub(super) fn get_capabilities(&self) -> Vec<String> {
        vec![
            String::from("actions"),
            String::from("body"),
            String::from("icon-static"),
            String::from("persistence"),
        ]
    }

    pub(super) fn get_server_information(&self) -> ServerInformation {
        self.server_information.clone()
    }

    /// Notify method
    #[allow(clippy::too_many_arguments)]
    pub(super) fn notify(
        &self,
        app_name: &str,
        replaces_id: u32,
        app_icon: &str,
        summary: &str,
        body: &str,
        actions: Vec<&str>,
        _hints: HashMap<&str, Value<'_>>,
        expire_timeout: i32,
    ) -> u32 {
        // creating an server ID for notification
        let id = self.current_id.fetch_add(1, Ordering::AcqRel);

        if let Err(err) = self.sender.send(SendType::Object(Request::Notification(
            NotificationRequests::New(ForwardedNotification {
                remote_id: id,
                app_name: app_name.to_string(),
                replaces_id,
                app_icon: app_icon.to_string(),
                summary: summary.to_string(),
                body: body.to_string(),
                actions: actions.into_iter().map(String::from).collect(),
                expire_timeout,
            }),
        ))) {
            error!("failed to forward notification to client {:?}", err);
        };

        id
    }

    #[zbus(signal)]
    pub(super) async fn notification_closed(
        emitter: &SignalEmitter<'_>,
        id: u32,
        reason: CloseReason,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    pub(super) async fn action_invoked(
        emitter: &SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}
