use std::collections::BTreeMap;

use anyhow::{Error, Result};

use crate::{
    rpc::{SendableMessage, Target},
    server::{RaftServer, ServerId},
};

pub trait TransportMedium<T> {
    fn send(&mut self, msg: &SendableMessage<T>) -> Result<()>;
}

pub struct ReliableTransport<'t, T> {
    peers: BTreeMap<ServerId, &'t mut RaftServer<'t, T>>,
}

/// Simulate a perfectly reliable transport medium that never drops packets
impl<'t, T> TransportMedium<T> for ReliableTransport<'t, T>
where
    T: Clone,
{
    fn send(&mut self, msg: &SendableMessage<T>) -> Result<()> {
        match msg {
            (Target::Single(target), rpc) => {
                // get target peer, return an error if its not found
                let peer = self
                    .peers
                    .get_mut(&target)
                    .ok_or(Error::msg("peer not found"))?;
                peer.receive_rpc(rpc);
                Ok(())
            }
            (Target::Broadcast, rpc) => {
                // broadcast this message to all peers
                self.peers
                    .values_mut()
                    .for_each(|peer| peer.receive_rpc(rpc));
                Ok(())
            }
        }
    }
}
