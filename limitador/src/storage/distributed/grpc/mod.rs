use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::Add;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{error::Error, io::ErrorKind, pin::Pin};

use moka::sync::Cache;
use tokio::sync::mpsc::Sender;
use tokio::sync::{broadcast, mpsc, RwLock};
use tokio::time::sleep;

use tokio_stream::{wrappers::ReceiverStream, Stream, StreamExt};
use tonic::{Code, Request, Response, Status, Streaming};

use crate::counter::Counter;
use crate::storage::distributed::cr_counter_value::CrCounterValue;
use crate::storage::distributed::grpc::v1::packet::Message;
use crate::storage::distributed::grpc::v1::replication_client::ReplicationClient;
use crate::storage::distributed::grpc::v1::replication_server::{Replication, ReplicationServer};
use crate::storage::distributed::grpc::v1::{
    CounterUpdate, Hello, MembershipUpdate, Packet, Peer, Pong,
};
use crate::storage::distributed::CounterKey;

// clippy will barf on protobuff generated code for enum variants in
// v3::socket_option::SocketState, so allow this lint
#[allow(clippy::enum_variant_names, clippy::derive_partial_eq_without_eq)]
pub mod v1 {
    tonic::include_proto!("limitador.service.distributed.v1");
}

#[derive(Copy, Clone, Debug)]
enum ClockSkew {
    None(),
    Slow(Duration),
    Fast(Duration),
}

impl ClockSkew {
    fn new(local: SystemTime, remote: SystemTime) -> ClockSkew {
        if local == remote {
            ClockSkew::None()
        } else if local.gt(&remote) {
            ClockSkew::Slow(local.duration_since(remote).unwrap())
        } else {
            ClockSkew::Fast(remote.duration_since(local).unwrap())
        }
    }

    #[allow(dead_code)]
    fn remote(&self, time: SystemTime) -> SystemTime {
        match self {
            ClockSkew::None() => time,
            ClockSkew::Slow(duration) => time - *duration,
            ClockSkew::Fast(duration) => time + *duration,
        }
    }

    #[allow(dead_code)]
    fn remote_now(&self) -> SystemTime {
        self.remote(SystemTime::now())
    }
}

impl std::fmt::Display for ClockSkew {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ClockSkew::None() => write!(f, "remote time is the same"),
            ClockSkew::Slow(duration) => {
                write!(f, "remote time is slow by {}ms", duration.as_millis())
            }
            ClockSkew::Fast(duration) => {
                write!(f, "remote time is fast by {}ms", duration.as_millis())
            }
        }
    }
}

#[derive(Clone)]
struct Session {
    broker_state: BrokerState,
    replication_state: Arc<RwLock<ReplicationState>>,
    out_stream: MessageSender,
    peer_id: String,
}

impl Session {
    async fn close(&mut self) {
        let mut state = self.replication_state.write().await;
        if let Some(peer) = state.peer_trackers.get_mut(&self.peer_id) {
            peer.session = None;
        }
    }

    async fn send(&self, message: Message) -> Result<(), Status> {
        self.out_stream.clone().send(Ok(message)).await
    }

    async fn process(&mut self, in_stream: &mut Streaming<Packet>) -> Result<(), Status> {
        // Send a MembershipUpdate to inform the peer about all the members
        // We should resend it again if we learn of new members.
        self.send(Message::MembershipUpdate(MembershipUpdate {
            peers: {
                let state = self.replication_state.read().await;
                state.peers().clone()
            },
        }))
        .await?;

        let mut udpates_to_send = self.broker_state.publisher.subscribe();

        loop {
            tokio::select! {
                update = udpates_to_send.recv() => {
                    let update = update.map_err(|_| Status::unknown("broadcast error"))?;
                    self.send(Message::CounterUpdate(update)).await?;
                }
                result = in_stream.next() => {
                    match result {
                        None=> {
                            // signals the end of stream...
                            return Ok(())
                        },
                        Some(Ok(packet)) => {
                            self.process_packet(packet).await?;
                        },
                        Some(Err(err)) => {
                            if is_disconnect(&err) {
                                println!("peer: '{}': disconnected: {:?}", self.peer_id, err);
                                return Ok(());
                            } else {
                                return Err(err);
                            }
                        },
                    }
                }
            }
        }
    }

    async fn process_packet(&self, packet: Packet) -> Result<(), Status> {
        match packet.message {
            Some(Message::Ping(_)) => {
                println!("peer: '{}': Ping", self.peer_id);
                self.out_stream
                    .clone()
                    .send(Ok(Message::Pong(Pong {
                        current_time: SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap()
                            .as_millis() as u64,
                    })))
                    .await?;
            }
            Some(Message::MembershipUpdate(update)) => {
                println!("peer: '{}': MembershipUpdate", self.peer_id);
                // add any new peers to peer_trackers
                let mut state = self.replication_state.write().await;
                for peer in update.peers {
                    let peer_id = peer.peer_id.clone();
                    match state.peer_trackers.get(&peer_id) {
                        None => {
                            // we are discovering a peer from neighbor, adding a tracker will
                            // trigger a connection attempt to it.
                            state.peer_trackers.insert(
                                peer_id.clone(),
                                PeerTracker {
                                    peer_id,
                                    url: None,
                                    discovered_urls: peer.urls.iter().cloned().collect(),
                                    latency: 0, // todo maybe set this to peer.latency + session.latency
                                    clock_skew: ClockSkew::None(),
                                    session: None,
                                },
                            );
                        }
                        Some(_peer_tracker) => {
                            // // TODO: add discovered urls to the existing tracker.
                            // peer.urls.clone().iter().for_each(|url| {
                            //     peer_tracker.discovered_urls.insert(url.clone());
                            // });
                        }
                    }
                }
            }
            Some(Message::CounterUpdate(update)) => {
                println!("peer: '{}': CounterUpdate", self.peer_id);

                let counter_key = postcard::from_bytes::<CounterKey>(update.key.as_slice())
                    .map_err(|err| {
                        Status::internal(format!("failed to decode counter key: {:?}", err))
                    })?;

                let values = BTreeMap::from_iter(
                    update
                        .values
                        .iter()
                        .map(|(k, v)| (k.to_owned(), v.to_owned())),
                );

                let counter = <CounterKey as Into<Counter>>::into(counter_key);
                if counter.is_qualified() {
                    if let Some(counter) = self.broker_state.qualified_counters.get(&counter) {
                        counter.merge(
                            (UNIX_EPOCH + Duration::from_secs(update.expires_at), values).into(),
                        );
                    }
                } else {
                    let counters = self.broker_state.limits_for_namespace.read().unwrap();
                    let limits = counters.get(counter.namespace()).unwrap();
                    let value = limits.get(counter.limit()).unwrap();
                    value.merge(
                        (UNIX_EPOCH + Duration::from_secs(update.expires_at), values).into(),
                    );
                };
            }
            _ => {
                println!("peer: '{}': unsupported packet: {:?}", self.peer_id, packet);
                return Err(Status::invalid_argument(format!(
                    "unsupported packet {:?}",
                    packet
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct PeerTracker {
    peer_id: String,
    url: Option<String>,
    discovered_urls: HashSet<String>,
    latency: u32,
    // Keep track of the clock skew between us and the peer
    clock_skew: ClockSkew,
    // The communication session we have with the peer, may be None if not connected
    session: Option<Session>,
}

// Track the replication session with all peers.
struct ReplicationState {
    // URLs our peers have used to connect to us.
    discovered_urls: HashSet<String>,
    peer_trackers: HashMap<String, PeerTracker>,
}

impl ReplicationState {
    fn peers(&self) -> Vec<Peer> {
        let mut peers = Vec::new();
        self.peer_trackers.iter().for_each(|(_, peer_tracker)| {
            peers.push(Peer {
                peer_id: peer_tracker.peer_id.clone(),
                latency: peer_tracker.latency,
                urls: peer_tracker
                    .discovered_urls
                    .iter()
                    .map(String::to_owned)
                    .collect(), // peer_tracker.urls.clone().into_iter().collect()
            });
        });
        peers.sort_by(|a, b| a.peer_id.cmp(&b.peer_id));
        peers
    }
}

fn match_for_io_error(err_status: &Status) -> Option<&std::io::Error> {
    let mut err: &(dyn Error + 'static) = err_status;

    loop {
        if let Some(io_err) = err.downcast_ref::<std::io::Error>() {
            return Some(io_err);
        }

        // h2::Error do not expose std::io::Error with `source()`
        // https://github.com/hyperium/h2/pull/462
        if let Some(h2_err) = err.downcast_ref::<h2::Error>() {
            if let Some(io_err) = h2_err.get_io() {
                return Some(io_err);
            }
        }

        err = match err.source() {
            Some(err) => err,
            None => return None,
        };
    }
}

async fn read_hello(in_stream: &mut Streaming<Packet>) -> Result<Hello, Status> {
    match in_stream.next().await {
        Some(Ok(packet)) => match packet.message {
            Some(Message::Hello(value)) => Ok(value),
            _ => Err(Status::invalid_argument("expected Hello")),
        },
        _ => Err(Status::invalid_argument("expected Hello")),
    }
}

async fn read_pong(in_stream: &mut Streaming<Packet>) -> Result<Pong, Status> {
    match in_stream.next().await {
        Some(Ok(packet)) => match packet.message {
            Some(Message::Pong(value)) => Ok(value),
            _ => Err(Status::invalid_argument("expected Pong")),
        },
        _ => Err(Status::invalid_argument("expected Pong")),
    }
}

fn is_disconnect(err: &Status) -> bool {
    if let Some(io_err) = match_for_io_error(err) {
        if io_err.kind() == ErrorKind::BrokenPipe {
            return true;
        }
    }
    false
}

// MessageSender is used to abstract the difference between the server and client sender streams...
#[derive(Clone)]
enum MessageSender {
    Server(Sender<Result<Packet, Status>>),
    Client(Sender<Packet>),
}

impl MessageSender {
    async fn send(self, message: Result<Message, Status>) -> Result<(), Status> {
        match self {
            MessageSender::Server(sender) => {
                let value = message.map(|x| Packet { message: Some(x) });
                let result = sender.send(value).await;
                result.map_err(|_| Status::unknown("send error"))
            }
            MessageSender::Client(sender) => match message {
                Ok(message) => {
                    let result = sender
                        .send(Packet {
                            message: Some(message),
                        })
                        .await;
                    result.map_err(|_| Status::unknown("send error"))
                }
                Err(err) => Err(err),
            },
        }
    }
}

#[derive(Clone)]
struct BrokerState {
    id: String,
    limits_for_namespace: Arc<std::sync::RwLock<super::LimitsMap>>,
    qualified_counters: Arc<Cache<Counter, Arc<CrCounterValue<String>>>>,
    publisher: broadcast::Sender<CounterUpdate>,
}

#[derive(Clone)]
pub struct Broker {
    listen_address: SocketAddr,
    peer_urls: Vec<String>,
    broker_state: BrokerState,
    replication_state: Arc<RwLock<ReplicationState>>,
}

impl Broker {
    pub fn new(
        id: String,
        listen_address: SocketAddr,
        peer_urls: Vec<String>,
        limits_for_namespace: Arc<std::sync::RwLock<super::LimitsMap>>,
        qualified_counters: Arc<Cache<Counter, Arc<CrCounterValue<String>>>>,
    ) -> Broker {
        let (tx, _) = broadcast::channel(16);
        let publisher: broadcast::Sender<CounterUpdate> = tx;

        Broker {
            listen_address,
            peer_urls,
            broker_state: BrokerState {
                id,
                publisher,
                limits_for_namespace,
                qualified_counters,
            },
            replication_state: Arc::new(RwLock::new(ReplicationState {
                discovered_urls: HashSet::new(),
                peer_trackers: HashMap::new(),
            })),
        }
    }

    pub fn publish(&self, counter_update: CounterUpdate) {
        // ignore the send error, it just means there are no active subscribers
        _ = self.broker_state.publisher.send(counter_update);
    }

    pub async fn start(&self) {
        self.clone().peer_urls.into_iter().for_each(|peer_url| {
            let broker = self.clone();
            let peer_url = peer_url.clone();
            _ = tokio::spawn(async move {
                // Keep trying until we get once successful connection handshake.  Once that
                // happens, we will know the peer_id and can recover by reconnecting to the peer
                loop {
                    match broker.connect_to_peer(peer_url.clone()).await {
                        Ok(_) => return,
                        Err(err) => {
                            println!("failed to connect with peer '{}': {:?}", peer_url, err);
                            sleep(Duration::from_secs(1)).await
                        }
                    }
                }
            });
        });

        // Periodically reconnect to failed peers
        {
            let broker = self.clone();
            tokio::spawn(async move {
                loop {
                    sleep(Duration::from_secs(1)).await;
                    broker.reconnect_to_failed_peers().await;
                }
            });
        }

        // // Simulate sending counter updates
        // {
        //     let server = server.clone();
        //     tokio::spawn(async move {
        //         loop {
        //             time::sleep(Duration::from_secs(1)).await;
        //
        //             let window = Duration::from_secs(10);
        //             server.publish_counter_update(CounterUpdate {
        //                 key: "my-counter".to_string(),
        //                 values: vec![74],
        //                 expires_at: SystemTime::now().add(window).duration_since(UNIX_EPOCH).unwrap().as_millis() ,
        //                 window: window.as_millis() as u64,
        //             }).await;
        //         }
        //     });
        // }

        println!(
            "peer '{}' listening on: id={}",
            self.broker_state.id, self.listen_address
        );

        tonic::transport::Server::builder()
            .add_service(ReplicationServer::new(self.clone()))
            .serve(self.listen_address)
            .await
            .unwrap();
    }

    // Connect to a peer and start a replication session.  This returns once the session handshake
    // completes.
    async fn connect_to_peer(&self, peer_url: String) -> Result<(), Status> {
        let mut client = match ReplicationClient::connect(peer_url.clone()).await {
            Ok(client) => client,
            Err(err) => {
                return Err(Status::new(Code::Unknown, err.to_string()));
            }
        };

        let (tx, rx) = mpsc::channel(1);

        let mut in_stream = client.stream(ReceiverStream::new(rx)).await?.into_inner();
        let mut sender = MessageSender::Client(tx);
        let session = self
            .handshake(&mut in_stream, &mut sender, Some(peer_url))
            .await?;

        // this means we already have a session with this peer...
        let mut session = match session {
            None => return Ok(()), // this just means we already have a session with this peer
            Some(session) => session,
        };

        // Session is now established, process the session async...
        tokio::spawn(async move {
            match session.process(&mut in_stream).await {
                Ok(_) => {
                    println!("client initiated stream ended");
                }
                Err(err) => {
                    println!("client initiated stream processing failed {:?}", err);
                }
            }
            session.close().await;
        });

        Ok(())
    }

    // Reconnect failed peers periodically
    async fn reconnect_to_failed_peers(&self) {
        let failed_peers: Vec<_> = {
            let state = self.replication_state.read().await;
            state
                .peer_trackers
                .iter()
                .filter_map(|(_, peer_tracker)| {
                    if peer_tracker.session.is_none() {
                        // first try to connect to the configured URL
                        let mut urls: Vec<_> = peer_tracker.url.iter().cloned().collect();
                        // Then try to connect to discovered urls.
                        let mut discovered_urls =
                            peer_tracker.discovered_urls.iter().cloned().collect();
                        urls.append(&mut discovered_urls);
                        Some((peer_tracker.peer_id.clone(), urls))
                    } else {
                        None
                    }
                })
                .collect()
        };

        for (peer_id, urls) in failed_peers {
            for url in urls {
                println!(
                    "attempting to reconnect to failed peer '{}' over {:?}",
                    peer_id, url
                );
                match self.connect_to_peer(url.clone()).await {
                    Ok(_) => break,
                    Err(err) => {
                        println!("failed to connect with peer '{}': {:?}", url, err);
                    }
                }
            }
        }
    }

    // handshake is called when a new stream is created, it will handle the initial handshake
    // and updating the session state in the state.peer_trackers map.  Result is None if an
    // existing session is already established with the peer.
    async fn handshake(
        &self,
        in_stream: &mut Streaming<Packet>,
        out_stream: &mut MessageSender,
        peer_url: Option<String>,
    ) -> Result<Option<Session>, Status> {
        // Let the peer know who we are...
        let start = SystemTime::now(); // .duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
        {
            let state = self.replication_state.read().await;
            out_stream
                .clone()
                .send(Ok(Message::Hello(Hello {
                    sender_peer_id: self.broker_state.id.clone(),
                    sender_urls: state.discovered_urls.clone().into_iter().collect(),
                    receiver_url: peer_url.clone(),
                })))
                .await?;
        }

        // Wait for the peer to tell us who he is...
        let peer_hello = read_hello(in_stream).await?;

        // respond with a Pong so the peer can calculate the round trip latency
        out_stream
            .clone()
            .send(Ok(Message::Pong(Pong {
                current_time: start.duration_since(UNIX_EPOCH).unwrap().as_millis() as u64,
            })))
            .await?;

        // Get the pong back from the peer...
        let peer_pong = read_pong(in_stream).await?;
        let end = SystemTime::now();

        let peer_id = peer_hello.sender_peer_id.clone();

        // When a peer initiates a connection, we discover a URL that can be used
        // to connect to us.
        if let Some(url) = peer_hello.receiver_url {
            let mut state = self.replication_state.write().await;
            state.discovered_urls.insert(url);
        }

        let session = Session {
            peer_id: peer_id.clone(),
            replication_state: self.replication_state.clone(),
            broker_state: self.broker_state.clone(),
            out_stream: out_stream.clone(),
        };

        // We now know who the peer is and our latency to him.
        let mut state = self.replication_state.write().await;
        let (tracker, option) = match state.peer_trackers.get_mut(&peer_id) {
            Some(tracker) => {
                match tracker.clone().session {
                    Some(prev_session) => {
                        // we already have a session with this peer, this is common since
                        // both peers are racing to connect to each other at the same time
                        // But we only need to keep one session.  Use the order of the
                        // peer ids to agree on which session keep.

                        if peer_id < self.broker_state.id {
                            // close the previous session, use the new one...
                            _ = prev_session
                                .out_stream
                                .send(Err(Status::already_exists("session")))
                                .await;
                            tracker.session = Some(session.clone());

                            (tracker, Some(session))
                        } else {
                            // use the previous session, close the new one...
                            _ = session
                                .out_stream
                                .send(Err(Status::already_exists("session")))
                                .await;
                            (tracker, None)
                        }
                    }
                    None => {
                        tracker.session = Some(session.clone());
                        (tracker, Some(session))
                    }
                }
            }
            None => {
                let latency = end.duration_since(start).unwrap();
                let peer_time = UNIX_EPOCH.add(Duration::from_millis(peer_pong.current_time));
                let peer_time_adj = peer_time.add(latency.div_f32(2.0)); // adjust for round trip latency
                let discovered_urls = peer_hello
                    .sender_urls
                    .iter()
                    .map(String::to_owned)
                    .collect();
                let tracker = PeerTracker {
                    peer_id: peer_id.clone(),
                    url: None,
                    discovered_urls,
                    latency: latency.as_millis() as u32,
                    clock_skew: ClockSkew::new(end, peer_time_adj),
                    session: Some(session.clone()),
                };

                println!(
                    "peer {} clock skew: {}",
                    peer_id.clone(),
                    &tracker.clock_skew
                );
                state.peer_trackers.insert(peer_id.clone(), tracker);
                let tracker = state.peer_trackers.get_mut(&peer_id).unwrap();
                (tracker, Some(session))
            }
        };

        // keep track of the URL we used to connect to the peer.
        if peer_url.is_some() {
            tracker.url.clone_from(&peer_url)
        }

        Ok(option)
    }
}

#[tonic::async_trait]
impl Replication for Broker {
    type StreamStream = Pin<Box<dyn Stream<Item = Result<Packet, Status>> + Send>>;

    // Accepts a connection from a peer and starts a replication session
    async fn stream(
        &self,
        req: Request<Streaming<Packet>>,
    ) -> Result<Response<Self::StreamStream>, Status> {
        println!("ReplicationServer::stream");

        let mut in_stream = req.into_inner();
        let (tx, rx) = mpsc::channel(1);

        let broker = self.clone();
        tokio::spawn(async move {
            let mut sender = MessageSender::Server(tx);
            match broker.handshake(&mut in_stream, &mut sender, None).await {
                Ok(Some(mut session)) => {
                    match session.process(&mut in_stream).await {
                        Ok(_) => {
                            println!("server accepted stream ended");
                        }
                        Err(err) => {
                            println!("server accepted stream processing failed {:?}", err);
                        }
                    }
                    session.close().await;
                }
                Ok(None) => {
                    // dup session..
                }
                Err(err) => {
                    println!("stream handshake failed {:?}", err);
                }
            }
        });

        Ok(Response::new(
            Box::pin(ReceiverStream::new(rx)) as Self::StreamStream
        ))
    }
}
