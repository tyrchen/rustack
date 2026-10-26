//! One bounded watcher per accepted physical call, retained through cleanup.

use tokio::sync::{mpsc, watch};

use super::{Message, WatchExit};
use crate::{
    FrameCallId, TaskEvent, TaskFailure, TaskSettlement, engine::runtime::actor::OwnedCall,
};

pub(super) async fn run(
    id: FrameCallId,
    mut owned: OwnedCall,
    mut cancelled: watch::Receiver<bool>,
    sender: mpsc::Sender<Message>,
) -> WatchExit {
    loop {
        if *cancelled.borrow() {
            break;
        }
        let event =
            tokio::select! {biased;_ = cancelled.changed()=>break,event=owned.call.next()=>event};
        let (message, done) = match event {
            Ok(TaskEvent::Started) => (Message::Started(id), false),
            Ok(TaskEvent::Completed(result)) => (Message::Completed(id, result), true),
            Err(_) => (
                Message::Completed(id, Err(TaskFailure::service_exception())),
                true,
            ),
        };
        let sent = tokio::select! {biased;_=cancelled.changed()=>false,result=sender.send(message)=>result.is_ok()};
        if !sent || done {
            break;
        }
    }
    owned.call.cancel();
    if owned.call.settlement() == TaskSettlement::Quarantined {
        let _ = sender.try_send(Message::Quarantined(id));
    }
    let clean = owned.call.settled().await.is_ok();
    WatchExit { id, owned, clean }
}
