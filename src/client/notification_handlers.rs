use tracing::{error, instrument};

use crate::{
    dbus::NotificationSignals,
    serialization::{Event, NotificationEvents, SendType},
};

use super::WprsClientState;

impl WprsClientState {
    #[instrument(skip(self), level = "debug")]
    pub fn handle_notification_signal(&mut self, signal: NotificationSignals) {
        let notification_id_mapper = self.notification_id_mapper.clone();
        let event_writer = self.serializer.writer().clone().into_inner();
        match signal {
            NotificationSignals::Close(local_notification_id) => {
                if let Err(err) = self.notification_scheduler.schedule(async move {
                    let notification_mapper = notification_id_mapper.lock().await;
                    if let Some(remote_notification_id) =
                        notification_mapper.get(&local_notification_id)
                    {
                        if let Err(err) = event_writer.send(SendType::Object(Event::Notification(
                            NotificationEvents::Closed(*remote_notification_id),
                        ))) {
                            error!("failed to send notification close event: {:?}", err);
                        };
                    };
                }) {
                    error!("failed to schedule notification close handle: {:?}", err);
                };
            },
            NotificationSignals::Action(local_notification_id, action_key) => {
                if let Err(err) = self.notification_scheduler.schedule(async move {
                    let notification_mapper = notification_id_mapper.lock().await;
                    if let Some(remote_notification_id) =
                        notification_mapper.get(&local_notification_id)
                    {
                        if let Err(err) = event_writer.send(SendType::Object(Event::Notification(
                            NotificationEvents::ActionInvoked(*remote_notification_id, action_key),
                        ))) {
                            error!("failed to send notification close event: {:?}", err);
                        };
                    };
                }) {
                    error!("failed to schedule notification close handle: {:?}", err);
                };
            },
        }
    }
}
