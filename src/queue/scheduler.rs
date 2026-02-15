use std::collections::{HashSet, VecDeque};

use super::IncomingMessage;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OrderingKey {
    WorkflowRun(String),
    Conversation {
        channel: String,
        channel_profile_id: String,
        conversation_id: String,
    },
    Message(String),
}

pub fn derive_ordering_key(payload: &IncomingMessage) -> OrderingKey {
    if let Some(workflow_run_id) = payload
        .workflow_run_id
        .as_ref()
        .filter(|s| !s.trim().is_empty())
    {
        return OrderingKey::WorkflowRun(workflow_run_id.clone());
    }

    if let (Some(channel_profile_id), Some(conversation_id)) = (
        payload
            .channel_profile_id
            .as_ref()
            .filter(|s| !s.trim().is_empty()),
        payload
            .conversation_id
            .as_ref()
            .filter(|s| !s.trim().is_empty()),
    ) {
        return OrderingKey::Conversation {
            channel: payload.channel.clone(),
            channel_profile_id: channel_profile_id.clone(),
            conversation_id: conversation_id.clone(),
        };
    }

    OrderingKey::Message(payload.message_id.clone())
}

#[derive(Debug)]
pub struct Scheduled<T> {
    pub key: OrderingKey,
    pub value: T,
}

#[derive(Debug)]
pub struct PerKeyScheduler<T> {
    pending: VecDeque<Scheduled<T>>,
    active_keys: HashSet<OrderingKey>,
}

impl<T> Default for PerKeyScheduler<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            active_keys: HashSet::new(),
        }
    }
}

impl<T> PerKeyScheduler<T> {
    pub fn enqueue(&mut self, key: OrderingKey, value: T) {
        self.pending.push_back(Scheduled { key, value });
    }

    pub fn dequeue_runnable(&mut self, max_items: usize) -> Vec<Scheduled<T>> {
        if max_items == 0 || self.pending.is_empty() {
            return Vec::new();
        }

        let mut selected = Vec::new();
        let mut selected_keys = HashSet::new();
        let mut remaining = VecDeque::new();

        while let Some(item) = self.pending.pop_front() {
            let key_busy =
                self.active_keys.contains(&item.key) || selected_keys.contains(&item.key);
            if !key_busy && selected.len() < max_items {
                selected_keys.insert(item.key.clone());
                self.active_keys.insert(item.key.clone());
                selected.push(item);
            } else {
                remaining.push_back(item);
            }
        }

        self.pending = remaining;
        selected
    }

    pub fn complete(&mut self, key: &OrderingKey) {
        self.active_keys.remove(key);
    }

    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub fn active_len(&self) -> usize {
        self.active_keys.len()
    }

    pub fn drain_pending(&mut self) -> Vec<Scheduled<T>> {
        self.pending.drain(..).collect()
    }
}
