//! The actual network seam: live TLS sockets and deterministic offline transport.
use crate::{
    net::{self, Error, RoomInfo, Visitor},
    proxy::Route,
};
use futures_util::{future::BoxFuture, FutureExt, SinkExt};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub trait MessageSocket: Send {
    fn read_frame<'a>(
        &'a mut self,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Vec<u8>>, Error>>;
    fn send_frame<'a>(
        &'a mut self,
        frame: Vec<u8>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>>;
}
pub trait Transport: Send + Sync {
    fn visitor<'a>(
        &'a self,
        route: &'a Route,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Visitor, Error>>;
    fn room<'a>(
        &'a self,
        route: &'a Route,
        visitor: &'a Visitor,
        rid: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<RoomInfo, Error>>;
    fn connect<'a>(
        &'a self,
        route: Arc<Route>,
        visitor: &'a Visitor,
        room: &'a str,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn MessageSocket>, Error>>;
}
pub struct LiveTransport;
impl Transport for LiveTransport {
    fn visitor<'a>(
        &'a self,
        r: &'a Route,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Visitor, Error>> {
        net::visitor(r, c).boxed()
    }
    fn room<'a>(
        &'a self,
        r: &'a Route,
        v: &'a Visitor,
        id: &'a str,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<RoomInfo, Error>> {
        net::room(r, v, id, c).boxed()
    }
    fn connect<'a>(
        &'a self,
        r: Arc<Route>,
        v: &'a Visitor,
        id: &'a str,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Box<dyn MessageSocket>, Error>> {
        async move { Ok(Box::new(net::connect(r, v, id, c).await?) as Box<dyn MessageSocket>) }
            .boxed()
    }
}
impl MessageSocket for net::Socket {
    fn read_frame<'a>(
        &'a mut self,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<Vec<u8>>, Error>> {
        net::receive(self, c).boxed()
    }
    fn send_frame<'a>(
        &'a mut self,
        frame: Vec<u8>,
        c: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move{
        tokio::select!{biased;_=c.cancelled()=>Err(Error::Cancelled),r=tokio::time::timeout(std::time::Duration::from_secs(5),SinkExt::send(self,tokio_tungstenite::tungstenite::Message::Binary(frame.into())))=>match r{Ok(Ok(()))=>Ok(()),_=>Err(Error::Network)}}
    }.boxed()
    }
}
