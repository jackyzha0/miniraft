use std::{collections::BTreeMap, fmt::Debug};

use anyhow::Result;

use crate::{
    rpc::{SendableMessage, Target},
    server::{RaftServer, ServerId},
};

pub trait TransportMedium<T> {
    /// Returns how many messages were passed in that tick
    fn tick(&mut self) -> Result<usize>;
}

pub struct ReliableTransport<T, S> {
    pub msg_queue: Vec<SendableMessage<T>>,
    pub peers: BTreeMap<ServerId, RaftServer<T, S>>,
}

/// Simulate a perfectly reliable transport medium that never drops packets
impl<T, S> TransportMedium<T> for ReliableTransport<T, S>
where
    T: Clone + Debug,
{
    fn tick(&mut self) -> Result<usize> {
        let old_msg_q_size = self.msg_queue.len();

        // iterate all peers and tick
        self.peers.values_mut().for_each(|peer| {
            let new_msgs = peer.tick();
            self.msg_queue.extend(new_msgs);
        });

        // send all things in msg queue
        let num_messages = self.msg_queue.len() - old_msg_q_size;
        let messages_to_send: Vec<SendableMessage<T>> = self.msg_queue.drain(..).collect();
        messages_to_send.iter().for_each(|msg| match msg {
            (Target::Single(target), rpc) => {
                // get target peer, return an error if its not found
                let peer = self.peers.get_mut(&target).expect("peer not found");
                let new_msgs = peer.receive_rpc(&rpc);
                self.msg_queue.extend(new_msgs);
            }
            (Target::Broadcast, rpc) => {
                // broadcast this message to all peers
                self.peers
                    .values_mut()
                    .map(|peer| peer.receive_rpc(&rpc))
                    .for_each(|new_msgs| self.msg_queue.extend(new_msgs));
            }
        });

        Ok(num_messages)
    }
}
