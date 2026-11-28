//! Vanilla STATUS plus the currently extracted LOGIN/CONFIGURATION route.

use crate::{
    login,
    play::{PlayServer, PreparedPlayWorld},
    registry_sync::RegistryBundle,
};
use horizon_protocol::{
    buf::Reader,
    connection::Connection,
    frame,
    packets::{
        Empty, handshake,
        status::{self, Ping, StatusResponse},
    },
};
use std::{
    io::{self, Read, Write},
    net::{TcpListener, TcpStream, ToSocketAddrs},
    sync::Arc,
    thread,
    time::Duration,
};

const VERSION_NAME: &str = "26.3";
const PROTOCOL_VERSION: i32 = 777;
const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);

/// Accept vanilla status pings indefinitely. Without a PLAY server, only
/// status is enabled and the response reports zero connected players.
pub(crate) fn listen(address: impl ToSocketAddrs) -> io::Result<()> {
    listen_inner(address, None, None, None, None)
}

/// Serve STATUS and offline LOGIN/CONFIGURATION. Connections close after
/// configuration until a PLAY runtime owns player sessions.
pub(crate) fn listen_with_login(
    address: impl ToSocketAddrs,
    registries: Arc<RegistryBundle>,
) -> io::Result<()> {
    listen_inner(address, Some(registries), None, None, None)
}

/// Serve STATUS and initialize a basic Overworld PLAY session. The simulation
/// remains incomplete; the prepared region is shared by all connecting clients.
pub(crate) fn listen_with_play_and_motd(
    address: impl ToSocketAddrs,
    registries: Arc<RegistryBundle>,
    world: Arc<PreparedPlayWorld>,
    motd: String,
    online: Option<Arc<login::OnlineAuthenticator>>,
) -> io::Result<()> {
    listen_inner(
        address,
        Some(registries),
        Some(Arc::new(PlayServer::new(world))),
        Some(motd),
        online,
    )
}

fn listen_inner(
    address: impl ToSocketAddrs,
    registries: Option<Arc<RegistryBundle>>,
    play_server: Option<Arc<PlayServer>>,
    motd: Option<String>,
    online: Option<Arc<login::OnlineAuthenticator>>,
) -> io::Result<()> {
    let listener = TcpListener::bind(address)?;
    let _world_tick_loop = play_server
        .as_ref()
        .map(|server| server.start_world_tick_loop())
        .transpose()?;
    println!(
        "Horizon protocol listener bound to {}",
        listener.local_addr()?
    );
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            // The tick-loop guard stops, joins, and flushes the world as this
            // listener returns instead of leaving a detached simulation thread.
            Err(error) => return Err(error),
        };
        let registries = registries.clone();
        let play_server = play_server.clone();
        let motd = motd.clone();
        let online = online.clone();
        thread::Builder::new()
            .name("horizon-protocol-client".into())
            .spawn(move || {
                if let Err(error) = handle(stream, registries, play_server, motd, online) {
                    eprintln!("Horizon protocol connection: {error}");
                }
            })?;
    }
}

fn handle(
    mut stream: TcpStream,
    registries: Option<Arc<RegistryBundle>>,
    play_server: Option<Arc<PlayServer>>,
    motd: Option<String>,
    online: Option<Arc<login::OnlineAuthenticator>>,
) -> io::Result<()> {
    stream.set_read_timeout(Some(CONNECTION_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECTION_TIMEOUT))?;
    let frame = read_frame(&mut stream)?;
    let (id, payload) = packet_parts(&frame)?;
    let intention = handshake::decode_intention(id, payload, handshake::PROXY_HOST_LIMIT)
        .map_err(protocol_error)?;
    if intention.intent == handshake::LOGIN {
        let Some(registries) = registries else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LOGIN is disabled on the status-only listener",
            ));
        };
        let remote_ip = stream.peer_addr().ok().map(|address| address.ip());
        let connection = Connection::from_stream(stream).map_err(protocol_error)?;
        let configured = login::login_after_intention(
            connection,
            intention,
            &registries,
            online.as_deref(),
            remote_ip,
        )
        .map_err(protocol_error)?;
        if let Some(server) = play_server {
            return server.initialize(configured).map_err(protocol_error);
        }
        println!(
            "Horizon: LOGIN/CONFIGURATION completed for {}; PLAY is not implemented yet",
            configured.profile.name
        );
        return Ok(());
    }
    if intention.intent != handshake::STATUS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported handshake intent {}", intention.intent),
        ));
    }

    match read_status_packet(&mut stream)? {
        status::Serverbound::StatusRequest(Empty) => {
            let online = play_server
                .as_ref()
                .map_or(0, |server| server.online_players());
            let max_players = play_server
                .as_ref()
                .map_or(20, |server| server.max_players());
            let json = serde_json::json!({
                "version": {"name": VERSION_NAME, "protocol": PROTOCOL_VERSION},
                "players": {"max": max_players, "online": online, "sample": []},
                "description": {"text": motd.as_deref().unwrap_or("Borizon Server")},
                "enforcesSecureChat": false,
            })
            .to_string();
            write_packet(
                &mut stream,
                &status::Clientbound::StatusResponse(StatusResponse { json }),
            )?;
        }
        packet => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("expected status request, got {}", packet.name()),
            ));
        }
    }

    if let status::Serverbound::PingRequest(ping) = read_status_packet(&mut stream)? {
        write_packet(
            &mut stream,
            &status::Clientbound::PongResponse(Ping { time: ping.time }),
        )?;
    }
    Ok(())
}

fn read_status_packet(stream: &mut TcpStream) -> io::Result<status::Serverbound> {
    let frame = read_frame(stream)?;
    let (id, payload) = packet_parts(&frame)?;
    status::Serverbound::decode(id, payload).map_err(protocol_error)
}

fn packet_parts(body: &[u8]) -> io::Result<(i32, &[u8])> {
    let mut reader = Reader::new(body);
    let id = reader.varint().map_err(protocol_error)?;
    Ok((id, &body[reader.position()..]))
}

fn read_frame(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut prefix = Vec::with_capacity(3);
    let length = loop {
        if prefix.len() == 3 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "packet frame length exceeds VarInt21",
            ));
        }
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        prefix.push(byte[0]);
        if let Some((prefix_len, length)) = frame::header(&prefix).map_err(protocol_error)? {
            if prefix_len != prefix.len() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid packet frame prefix",
                ));
            }
            break length;
        }
    };
    if length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("packet frame exceeds {MAX_FRAME_BYTES} bytes"),
        ));
    }
    let mut body = vec![0; length];
    stream.read_exact(&mut body)?;
    Ok(body)
}

fn write_packet(stream: &mut TcpStream, packet: &status::Clientbound) -> io::Result<()> {
    let body = packet.encode();
    let framed = frame::encode(&body, None).map_err(protocol_error)?;
    stream.write_all(&framed)
}

fn protocol_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry_sync::{RegistryBundle, SYNCHRONIZED_REGISTRIES};
    use horizon_protocol::{
        connection::Connection,
        packets::{
            Empty, configuration,
            handshake::{self, Intention},
            login,
            status::{Clientbound, Ping},
        },
    };
    use horizon_world::nbt::Tag;

    #[test]
    fn status_packet_uses_the_pinned_26_3_protocol() {
        let packet = Clientbound::StatusResponse(StatusResponse {
            json: format!(
                "{{\"version\":{{\"name\":\"{VERSION_NAME}\",\"protocol\":{PROTOCOL_VERSION}}}}}"
            ),
        });
        assert_eq!(
            packet.id(),
            horizon_protocol::ids::status::clientbound::STATUS_RESPONSE
        );
        let pong = Clientbound::PongResponse(Ping { time: 123 });
        assert_eq!(
            pong.id(),
            horizon_protocol::ids::status::clientbound::PONG_RESPONSE
        );
    }

    #[test]
    fn tcp_listener_routes_login_and_completes_configuration() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let registries = Arc::new(RegistryBundle::from_test_parts(
            SYNCHRONIZED_REGISTRIES
                .iter()
                .map(|registry| configuration::RegistryData {
                    registry: (*registry).into(),
                    entries: vec![configuration::RegistryEntry {
                        id: "minecraft:test".into(),
                        data: Some(Tag::compound([("fixture", Tag::Byte(1))])),
                    }],
                })
                .collect(),
            Vec::new(),
        ));
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            handle(stream, Some(registries), None, None, None).unwrap();
        });

        let stream = TcpStream::connect(address).unwrap();
        let mut client = Connection::from_stream(stream).unwrap();
        client
            .send(
                &handshake::Serverbound::Intention(Intention {
                    protocol: PROTOCOL_VERSION,
                    host: "localhost".into(),
                    port: address.port(),
                    intent: handshake::LOGIN,
                })
                .encode(),
            )
            .unwrap();
        client
            .send(
                &login::Serverbound::Hello(login::ServerHello {
                    name: "TcpPlayer".into(),
                    uuid: 0,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = client.recv().unwrap();
        assert!(matches!(
            login::Clientbound::decode(id, &payload).unwrap(),
            login::Clientbound::LoginFinished(_)
        ));
        client
            .send(&login::Serverbound::LoginAcknowledged(Empty).encode())
            .unwrap();

        for expected in ["features", "known packs"] {
            let (id, payload) = client.recv().unwrap();
            let packet = configuration::Clientbound::decode(id, &payload).unwrap();
            match (expected, packet) {
                ("features", configuration::Clientbound::UpdateEnabledFeatures(_))
                | ("known packs", configuration::Clientbound::SelectKnownPacks(_)) => {}
                (_, packet) => panic!("expected configuration {expected}, got {packet:?}"),
            }
        }
        client
            .send(
                &configuration::Serverbound::SelectKnownPacks(configuration::SelectKnownPacks {
                    packs: Vec::new(),
                })
                .encode(),
            )
            .unwrap();

        let mut seen_registries = Vec::new();
        loop {
            let (id, payload) = client.recv().unwrap();
            match configuration::Clientbound::decode(id, &payload).unwrap() {
                configuration::Clientbound::RegistryData(registry) => {
                    seen_registries.push(registry.registry);
                }
                configuration::Clientbound::UpdateTags(_) => {}
                configuration::Clientbound::FinishConfiguration(_) => break,
                packet => panic!("unexpected configuration packet {packet:?}"),
            }
        }
        assert_eq!(seen_registries, SYNCHRONIZED_REGISTRIES);
        client
            .send(&configuration::Serverbound::FinishConfiguration(Empty).encode())
            .unwrap();
        server.join().unwrap();
    }
}
