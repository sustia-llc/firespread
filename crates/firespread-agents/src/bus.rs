//! Named mailboxes and the in-process bus that routes [`Delivery`] envelopes
//! to them.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex};

use kanal::{AsyncReceiver, AsyncSender};

use crate::{AgentError, Result};

/// One message addressed to the mailbox named `recipient`.
#[derive(Debug)]
pub struct Delivery<T> {
    pub recipient: String,
    pub message: T,
}

/// A router of [`Delivery`] envelopes to named mailboxes.
pub trait Bus<T: Send + 'static>: Send + Sync + 'static {
    /// Opens the mailbox `name` and returns its receiver. Every later
    /// delivery whose `recipient` is `name` arrives on it, in send order.
    ///
    /// # Errors
    /// [`AgentError::DuplicateAgent`] when `name` is already registered.
    fn register(&self, name: &str) -> Result<AsyncReceiver<Delivery<T>>>;

    /// Delivers `delivery` to the mailbox named `delivery.recipient`.
    ///
    /// # Errors
    /// [`AgentError::UnknownAgent`] when no mailbox carries that name, and
    /// [`AgentError::Undeliverable`] when the mailbox is closed.
    fn send(&self, delivery: Delivery<T>) -> impl Future<Output = Result<()>> + Send;

    /// Closes every mailbox and drops the messages queued in it; pending and
    /// later receives and sends on it fail.
    fn close(&self);
}

/// The in-process [`Bus`]: one unbounded kanal channel per mailbox.
pub struct KanalBus<T> {
    mailboxes: Mutex<HashMap<String, AsyncSender<Delivery<T>>>>,
}

impl<T> Default for KanalBus<T> {
    fn default() -> Self {
        Self {
            mailboxes: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> std::fmt::Debug for KanalBus<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<String> = self
            .mailboxes
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default();
        f.debug_struct("KanalBus")
            .field("mailboxes", &names)
            .finish()
    }
}

impl<T> KanalBus<T> {
    /// An empty bus.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn sender(&self, name: &str) -> Result<AsyncSender<Delivery<T>>> {
        let guard = self
            .mailboxes
            .lock()
            .map_err(|_| AgentError::Undeliverable {
                name: name.to_string(),
                message: "bus registry poisoned".to_string(),
            })?;
        guard
            .get(name)
            .cloned()
            .ok_or_else(|| AgentError::UnknownAgent {
                name: name.to_string(),
            })
    }
}

impl<T: Send + 'static> Bus<T> for KanalBus<T> {
    fn register(&self, name: &str) -> Result<AsyncReceiver<Delivery<T>>> {
        let mut guard = self
            .mailboxes
            .lock()
            .map_err(|_| AgentError::DuplicateAgent {
                name: name.to_string(),
            })?;
        if guard.contains_key(name) {
            return Err(AgentError::DuplicateAgent {
                name: name.to_string(),
            });
        }
        let (tx, rx) = kanal::unbounded_async();
        guard.insert(name.to_string(), tx);
        Ok(rx)
    }

    async fn send(&self, delivery: Delivery<T>) -> Result<()> {
        let tx = self.sender(&delivery.recipient)?;
        let name = delivery.recipient.clone();
        tx.send(delivery)
            .await
            .map_err(|_| AgentError::Undeliverable {
                name,
                message: "mailbox closed".to_string(),
            })
    }

    fn close(&self) {
        if let Ok(guard) = self.mailboxes.lock() {
            for tx in guard.values() {
                // Err means the channel is already closed, the state asked for.
                let _ = tx.close();
            }
        }
    }
}
