#![allow(dead_code)] //TODO: remove this after everything is done
//TODO: remove all unwraps
use std::net::{SocketAddr, AddrParseError};
use std::collections::{HashSet, HashMap, BTreeMap};
use std::hash::{Hash, Hasher};
use std::cell::{RefCell};
use std::ops::{DerefMut};
use std::str::{FromStr};
use mio::*;
use mio::util::{Slab};
use mio::tcp::*;
use mio::udp::*;
use hash::*;
use bytes::*;
use time::Tm;
use error::EthcoreError;

const DEFAULT_PORT: u16 = 30303;

const ADDRESS_BYTES_SIZE: u32 = 32;		        			///< Size of address type in bytes.
const ADDRESS_BITS: u32 = 8 * ADDRESS_BYTES_SIZE;			///< Denoted by n in [Kademlia].
const NODE_BINS: u32 = ADDRESS_BITS - 1;					///< Size of m_state (excludes root, which is us).
const DISCOVERY_MAX_STEPS: u16 = 8;	                        ///< Max iterations of discovery. (discover)
const MAX_CONNECTIONS: usize = 1024;
const IDEAL_PEERS:u32 = 10;

const BUCKET_SIZE: u32 = 16;	    ///< Denoted by k in [Kademlia]. Number of nodes stored in each bucket.
const ALPHA: usize = 3;				///< Denoted by \alpha in [Kademlia]. Number of concurrent FindNode requests.

type NodeId = H512;
type PublicKey = H512;
type SecretKey = H256;

#[derive(Debug)]
struct NetworkConfiguration {
    listen_address: SocketAddr,
    public_address: SocketAddr,
    no_nat: bool,
    no_discovery: bool,
    pin: bool,
}

impl NetworkConfiguration {
    fn new() -> NetworkConfiguration {
        NetworkConfiguration {
            listen_address: SocketAddr::from_str("0.0.0.0:30303").unwrap(),
            public_address: SocketAddr::from_str("0.0.0.0:30303").unwrap(),
            no_nat: false,
            no_discovery: false,
            pin: false
        }
    }
}

#[derive(Debug)]
struct NodeEndpoint {
    address: SocketAddr,
    udp_port: u16
}

impl NodeEndpoint {
    fn new(address: SocketAddr) -> NodeEndpoint {
        NodeEndpoint {
            address: address,
            udp_port: address.port()
        }
    }
    fn from_str(address: &str) -> Result<NodeEndpoint, AddrParseError> {
		let address = try!(SocketAddr::from_str(address));
        Ok(NodeEndpoint {
            address: address,
            udp_port: address.port()
        })
    }
}

#[derive(Debug)]
pub enum AddressError {
	AddrParseError(AddrParseError),
	NodeIdParseError(EthcoreError)
}

impl From<AddrParseError> for AddressError {
	fn from(err: AddrParseError) -> AddressError {
		AddressError::AddrParseError(err)
	}
}
impl From<EthcoreError> for AddressError {
	fn from(err: EthcoreError) -> AddressError {
		AddressError::NodeIdParseError(err)
	}
}

#[derive(PartialEq, Eq, Copy, Clone)]
enum PeerType {
    Required,
    Optional
}

struct Node {
    id: NodeId,
    endpoint: NodeEndpoint,
    peer_type: PeerType,
	last_attempted: Option<Tm>,
	confirmed: bool,
}

impl FromStr for Node {
	type Err = AddressError;
	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let (id, endpoint) = if &s[..8] == "enode://" && s.len() > 136 && &s[136..137] == "@" {
			(try!(NodeId::from_str(&s[8..128])), try!(NodeEndpoint::from_str(&s[137..])))
		}
		else {
			(NodeId::new(), try!(NodeEndpoint::from_str(s)))
		};

        Ok(Node {
            id: id,
            endpoint: endpoint,
            peer_type: PeerType::Optional,
			last_attempted: None,
			confirmed: false
        })
	}
}

impl Node {
    fn new(id: NodeId, address: SocketAddr, t:PeerType) -> Node {
        Node {
            id: id,
            endpoint: NodeEndpoint::new(address),
            peer_type: t,
			last_attempted: None,
			confirmed: false
        }
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl Eq for Node { }

impl Hash for Node {
    fn hash<H>(&self, state: &mut H) where H: Hasher {
        self.id.hash(state)
    }
}

struct NodeBucket {
    distance: u32,
    nodes: Vec<NodeId>
}

impl NodeBucket {
    fn new(distance: u32) -> NodeBucket {
        NodeBucket {
            distance: distance,
            nodes: Vec::new()
        }
    }
}

struct Connection {
    socket: TcpStream,
	send_queue: Vec<Bytes>,
}

impl Connection {
	fn new(socket: TcpStream) -> Connection {
		Connection {
			socket: socket,
			send_queue: Vec::new(),
		}
	}
}

#[derive(PartialEq, Eq)]
enum HandshakeState {
	New,
	AckAuth,
	WriteHello,
	ReadHello,
	StartSession,
}

struct Handshake {
	id: NodeId,
	connection: Connection,
	state: HandshakeState,
}

impl Handshake {
	fn new(id: NodeId, socket: TcpStream) -> Handshake {
		Handshake {
			id: id,
			connection: Connection::new(socket),
			state: HandshakeState::New
		}
	}
}

struct Peer {
	id: NodeId,
	connection: Connection,
}

struct FindNodePacket;

impl FindNodePacket {
    fn new(_endpoint: &NodeEndpoint, _id: &NodeId) -> FindNodePacket {
        FindNodePacket
    }
    fn sign(&mut self, _secret: &SecretKey) {
    }

    fn send(& self, _socket: &mut UdpSocket) {
    }
}

// Tokens
const TCP_ACCEPT: usize = 1;
const IDLE: usize = 3;
const NODETABLE_RECEIVE: usize = 4;
const NODETABLE_MAINTAIN: usize = 5;
const NODETABLE_DISCOVERY: usize = 6;
const FIRST_CONNECTION: usize = 7;
const LAST_CONNECTION: usize = FIRST_CONNECTION + MAX_CONNECTIONS - 1;
const FIRST_HANDSHAKE: usize = FIRST_CONNECTION + MAX_CONNECTIONS;
const LAST_HANDSHAKE: usize = FIRST_HANDSHAKE + MAX_CONNECTIONS - 1;

pub enum HostMessage {
    Shutdown
}

pub struct Host {
    secret: SecretKey,
    node: Node,
    sender: Sender<HostMessage>,
    config: NetworkConfiguration,
    udp_socket: UdpSocket,
    listener: TcpListener,
    peers: Slab<Peer>,
    connecting: Slab<Handshake>,
    discovery_round: u16,
    discovery_id: NodeId,
    discovery_nodes: HashSet<NodeId>,
    node_buckets: Vec<NodeBucket>,
	nodes: HashMap<NodeId, Node>,
	idle_timeout: Timeout,
}

impl Host {
    pub fn start() {
        let config = NetworkConfiguration::new();
		/*
		match ::ifaces::Interface::get_all().unwrap().into_iter().filter(|x| x.kind == ::ifaces::Kind::Packet && x.addr.is_some()).next() {
			Some(iface) => config.public_address = iface.addr.unwrap(),
			None => warn!("No public network interface"),
		}
		*/

        let addr = config.listen_address;
        // Setup the server socket
        let listener = TcpListener::bind(&addr).unwrap();
        // Create an event loop
        let mut event_loop = EventLoop::new().unwrap();
        let sender = event_loop.channel();
        // Start listening for incoming connections
        event_loop.register_opt(&listener, Token(TCP_ACCEPT), EventSet::readable(), PollOpt::edge()).unwrap();
        // Setup the client socket
        //let sock = TcpStream::connect(&addr).unwrap();
        // Register the socket
        //self.event_loop.register_opt(&sock, CLIENT, EventSet::readable(), PollOpt::edge()).unwrap();
        let idle_timeout = event_loop.timeout_ms(Token(IDLE), 1000).unwrap(); //TODO: check delay
        // open the udp socket
        let udp_socket = UdpSocket::bound(&addr).unwrap();
        event_loop.register_opt(&udp_socket, Token(NODETABLE_RECEIVE), EventSet::readable(), PollOpt::edge()).unwrap();
        event_loop.timeout_ms(Token(NODETABLE_MAINTAIN), 7200).unwrap();

        let mut host = Host {
            secret: SecretKey::new(),
            node: Node::new(NodeId::new(), config.public_address.clone(), PeerType::Required), 
            config: config,
            sender: sender,
            udp_socket: udp_socket,
            listener: listener,
			peers: Slab::new_starting_at(Token(FIRST_CONNECTION), MAX_CONNECTIONS),
			connecting: Slab::new_starting_at(Token(FIRST_HANDSHAKE), MAX_CONNECTIONS),
            discovery_round: 0,
            discovery_id: NodeId::new(),
            discovery_nodes: HashSet::new(),
            node_buckets: (0..NODE_BINS).map(|x| NodeBucket::new(x)).collect(),
			nodes: HashMap::new(),
			idle_timeout: idle_timeout
        };


		host.add_node("enode://5374c1bff8df923d3706357eeb4983cd29a63be40a269aaa2296ee5f3b2119a8978c0ed68b8f6fc84aad0df18790417daadf91a4bfbb786a16c9b0a199fa254a@gav.ethdev.com:30300");
		host.add_node("enode://e58d5e26b3b630496ec640f2530f3e7fa8a8c7dfe79d9e9c4aac80e3730132b869c852d3125204ab35bb1b1951f6f2d40996c1034fd8c5a69b383ee337f02dd@gav.ethdev.com:30303");
		host.add_node("enode://a979fb575495b8d6db44f750317d0f4622bf4c2aa3365d6af7c284339968eef29b69ad0dce72a4d8db5ebb4968de0e3bec910127f134779fbcb0cb6d3331163@52.16.188.185:30303");
		host.add_node("enode://7f25d3eab333a6b98a8b5ed68d962bb22c876ffcd5561fca54e3c2ef27f754df6f7fd7c9b74cc919067abac154fb8e1f8385505954f161ae440abc355855e03@54.207.93.166:30303");
		host.add_node("enode://5374c1bff8df923d3706357eeb4983cd29a63be40a269aaa2296ee5f3b2119a8978c0ed68b8f6fc84aad0df18790417daadf91a4bfbb786a16c9b0a199fa254@92.51.165.126:30303");

        event_loop.run(&mut host).unwrap();
    }

    fn stop(&mut self) {
    }

    fn have_network(&mut self) -> bool {
        true
    }

	fn add_node(&mut self, id: &str) {
		match Node::from_str(id) {
			Err(e) => { warn!("Could not add node: {:?}", e); },
			Ok(n) => { self.nodes.insert(n.id.clone(), n); }
		}
	}

    fn start_node_discovery(&mut self, event_loop: &mut EventLoop<Host>) {
        self.discovery_round = 0;
        self.discovery_id.randomize();
        self.discovery_nodes.clear();
        self.discover(event_loop);
    }

    fn discover(&mut self, event_loop: &mut EventLoop<Host>) {
        if self.discovery_round == DISCOVERY_MAX_STEPS
        {
            debug!("Restarting discovery");
            self.start_node_discovery(event_loop);
            return;
        }
        let mut tried_count = 0;
        {
            let nearest = Host::nearest_node_entries(&self.node.id, &self.discovery_id, &self.node_buckets).into_iter();
            let nodes = RefCell::new(&mut self.discovery_nodes);
            let nearest = nearest.filter(|x| nodes.borrow().contains(&x)).take(ALPHA);
            for r in nearest {
                //let mut p = FindNodePacket::new(&r.endpoint, &self.discovery_id);
                //p.sign(&self.secret);
                //p.send(&mut self.udp_socket);
                let mut borrowed = nodes.borrow_mut();
                borrowed.deref_mut().insert(r.clone());
                tried_count += 1;
            }
        }

        if tried_count == 0
        {
            debug!("Restarting discovery");
            self.start_node_discovery(event_loop);
            return;
        }
        self.discovery_round += 1;
        event_loop.timeout_ms(Token(NODETABLE_DISCOVERY), 1200).unwrap();
    }

	fn distance(a: &NodeId, b: &NodeId) -> u32 { 
        //TODO: 
        //u256 d = sha3(_a) ^ sha3(_b); 
        let mut d: NodeId = NodeId::new();
        for i in 0..32 {
            d[i] = a[i] ^ b[i];
        }
        
        let mut ret:u32 = 0;
        for i in 0..32 {
            let mut v: u8 = d[i];
            while v != 0 {
                v >>= 1;
                ret += 1;
            }
        }
        ret
    }

    fn nearest_node_entries<'a>(source: &NodeId, target: &NodeId, buckets: &'a Vec<NodeBucket>) -> Vec<&'a NodeId>
    {
        // send ALPHA FindNode packets to nodes we know, closest to target
        const LAST_BIN: u32 = NODE_BINS - 1;
        let mut head = Host::distance(source, target);
        let mut tail = if head == 0  { LAST_BIN } else { (head - 1) % NODE_BINS };

        let mut found: BTreeMap<u32, Vec<&'a NodeId>> = BTreeMap::new();
        let mut count = 0;

        // if d is 0, then we roll look forward, if last, we reverse, else, spread from d
        if head > 1 && tail != LAST_BIN {
            while head != tail && head < NODE_BINS && count < BUCKET_SIZE
            {
                for n in buckets[head as usize].nodes.iter()
                {
                        if count < BUCKET_SIZE {
							count += 1;
                            found.entry(Host::distance(target, &n)).or_insert(Vec::new()).push(n);
                        }
                        else {
                            break;
                        }
                }
                if count < BUCKET_SIZE && tail != 0 {
                    for n in buckets[tail as usize].nodes.iter() {
                        if count < BUCKET_SIZE {
							count += 1;
                            found.entry(Host::distance(target, &n)).or_insert(Vec::new()).push(n);
                        }
                        else {
                            break;
                        }
                    }
                }

                head += 1;
                if tail > 0 {
                    tail -= 1;
                }
            }
        }
        else if head < 2 {
            while head < NODE_BINS && count < BUCKET_SIZE {
                for n in buckets[head as usize].nodes.iter() {
                        if count < BUCKET_SIZE {
							count += 1;
                            found.entry(Host::distance(target, &n)).or_insert(Vec::new()).push(n);
                        }
                        else {
                            break;
                        }
                }
                head += 1;
            }
        }
        else {
            while tail > 0 && count < BUCKET_SIZE {
                for n in buckets[tail as usize].nodes.iter() {
                        if count < BUCKET_SIZE {
							count += 1;
                            found.entry(Host::distance(target, &n)).or_insert(Vec::new()).push(n);
                        }
                        else {
                            break;
                        }
                }
                tail -= 1;
            }
        }

        let mut ret:Vec<&NodeId> = Vec::new();
        for (_, nodes) in found {
            for n in nodes {
                if ret.len() < BUCKET_SIZE as usize /* && n->endpoint && n->endpoint.isAllowed() */ {
                    ret.push(n);
                }
            }
        }
        ret
    }

    fn maintain_network(&mut self, event_loop: &mut EventLoop<Host>) {
        self.keep_alive();
        self.connect_peers(event_loop);
    }

	fn have_session(&self, id: &NodeId) -> bool {
		self.peers.iter().any(|h| h.id.eq(&id))
	}

	fn connecting_to(&self, id: &NodeId) -> bool {
		self.connecting.iter().any(|h| h.id.eq(&id))
	}

    fn connect_peers(&mut self, event_loop: &mut EventLoop<Host>) {

		struct NodeInfo {
			id: NodeId,
			peer_type: PeerType
		}

		let mut to_connect: Vec<NodeInfo> = Vec::new();

		let mut req_conn = 0;
		for n in self.node_buckets.iter().flat_map(|n| &n.nodes).map(|id| NodeInfo { id: id.clone(), peer_type: self.nodes.get(id).unwrap().peer_type}) {
			let connected = self.have_session(&n.id) || self.connecting_to(&n.id);
			let required = n.peer_type == PeerType::Required;
			if connected && required {
				req_conn += 1;
			}
			else if !connected && (!self.config.pin || required) {
				to_connect.push(n);
			}
		}

		for n in to_connect.iter() {
			if n.peer_type == PeerType::Required {
				if req_conn < IDEAL_PEERS {
					self.connect_peer(&n.id, event_loop);
				}
				req_conn += 1;
			}
		}
		
		if !self.config.pin
		{
			let pending_count = 0; //TODO:
			let peer_count = 0;
			let mut open_slots = IDEAL_PEERS - peer_count  - pending_count + req_conn;
			if open_slots > 0 {
				for n in to_connect.iter() {
					if n.peer_type == PeerType::Optional && open_slots > 0 {
						open_slots -= 1;
						self.connect_peer(&n.id, event_loop);
					}
				}
			}
		}
    }

	fn connect_peer(&mut self, id: &NodeId, event_loop: &mut EventLoop<Host>) {
		if self.have_session(id)
		{
			warn!("Aborted connect. Node already connected.");
			return;
		}
		if self.connecting_to(id)
		{
			warn!("Aborted connect. Node already connecting.");
			return;
		}
		let node = self.nodes.get_mut(id).unwrap();
		node.last_attempted = Some(::time::now());
		
		
		//blog(NetConnect) << "Attempting connection to node" << _p->id << "@" << ep << "from" << id();
		let socket = match TcpStream::connect(&node.endpoint.address) {
			Ok(socket) => socket,
			Err(_) => {
				warn!("Cannot connect to node");
				return;
			}
		};
		let handshake = Handshake::new(id.clone(), socket);
		match self.connecting.insert(handshake) {
			Ok(token) => event_loop.register_opt(&self.connecting[token].connection.socket, token, EventSet::all(), PollOpt::edge()).unwrap(),
			Err(_) => warn!("Max connections reached")
		};
	}

    fn keep_alive(&mut self) {
    }



	fn accept(&mut self, _event_loop: &mut EventLoop<Host>) {
		warn!(target "net", "accept");
	}

	fn start_handshake(&mut self, token: Token,  _event_loop: &mut EventLoop<Host>) {
		let handshake = match self.handshakes.get(&token) {
			Some(h) => h,
			None => {
				warn!(target "net", "Received event for unknown handshake");
				return;
			}
		};




	}

	fn read_handshake(&mut self, _event_loop: &mut EventLoop<Host>) {
				warn!(target "net", "accept");
	}

	fn read_connection(&mut self, _event_loop: &mut EventLoop<Host>) {
	}

	fn write_connection(&mut self, _event_loop: &mut EventLoop<Host>) {
	}
}

impl Handler for Host {
    type Timeout = Token;
    type Message = HostMessage;

    fn ready(&mut self, event_loop: &mut EventLoop<Host>, token: Token, events: EventSet) {
        if events.is_readable() {
			match token.as_usize() {
				TCP_ACCEPT =>  self.accept(event_loop),
				IDLE => self.maintain_network(event_loop),
				FIRST_CONNECTION ... LAST_CONNECTION => self.read_connection(event_loop),
				FIRST_HANDSHAKE ... LAST_HANDSHAKE => self.read_handshake(event_loop),
				NODETABLE_RECEIVE => {},
				_ => panic!("Received unknown readable token"),
			}
		}
        else if events.is_writable() {
			match token.as_usize() {
				FIRST_CONNECTION ... LAST_CONNECTION => self.write_connection(event_loop),
				FIRST_HANDSHAKE ... LAST_HANDSHAKE => self.start_handshake(event_loop),
				_ => panic!("Received unknown writable token"),
			}
		}
    }

	fn timeout(&mut self, event_loop: &mut EventLoop<Host>, token: Token) {
		match token.as_usize() {
			IDLE => self.maintain_network(event_loop),
			NODETABLE_DISCOVERY => {},
			NODETABLE_MAINTAIN => {},
			_ => panic!("Received unknown timer token"),
		}
	}
}


#[cfg(test)]
mod tests {
    use network::host::Host;
    #[test]
	#[ignore]
    fn net_connect() {
        let _ = Host::start();
    }
}



