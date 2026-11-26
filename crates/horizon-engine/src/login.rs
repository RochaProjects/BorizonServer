//! Vanilla offline-mode LOGIN and the CONFIGURATION exchange.
//!
//! This module stops before PLAY: world ownership, player admission and the
//! initial chunk/light stream are still not implemented by the engine.

use crate::registry_sync::RegistryBundle;
use horizon_protocol::{
    connection::Connection,
    crypto::offline_uuid,
    packets::{
        configuration, handshake,
        login::{self, GameProfile, Property},
    },
};

const PROTOCOL_VERSION: i32 = 777;
const MAX_CONFIGURATION_PACKETS: usize = 256;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OfflineProfile {
    pub name: String,
    pub uuid: u128,
}

pub struct ConfiguredConnection {
    pub connection: Connection,
    pub profile: OfflineProfile,
    pub client_information: Option<configuration::ClientInformation>,
}

/// Complete the 26.3 handshake, offline LOGIN and vanilla CONFIGURATION.
/// This is only a connection primitive; callers must not announce the player
/// as joined until a PLAY runtime has admitted and initialized the session.
pub fn login_and_configure(
    mut connection: Connection,
    registries: &RegistryBundle,
) -> Result<ConfiguredConnection, String> {
    let (id, payload) = connection.recv().map_err(|error| error.to_string())?;
    let intention = handshake::decode_intention(id, &payload, 255)
        .map_err(|error| format!("decode Minecraft handshake: {error}"))?;
    login_after_intention(connection, intention, registries)
}

/// Continue a real TCP session after its handshake has been consumed by a
/// listener that also routes STATUS requests.
pub(crate) fn login_after_intention(
    mut connection: Connection,
    intention: handshake::Intention,
    registries: &RegistryBundle,
) -> Result<ConfiguredConnection, String> {
    if intention.protocol != PROTOCOL_VERSION {
        return Err(format!(
            "client protocol {} does not match Minecraft 26.3 ({PROTOCOL_VERSION})",
            intention.protocol
        ));
    }
    if intention.intent != handshake::LOGIN {
        return Err(format!("expected LOGIN intent, got {}", intention.intent));
    }

    let (id, payload) = connection.recv().map_err(|error| error.to_string())?;
    let hello = match login::Serverbound::decode(id, &payload)
        .map_err(|error| format!("decode LOGIN packet: {error}"))?
    {
        login::Serverbound::Hello(hello) => hello,
        packet => return Err(format!("expected login hello, got {}", packet.name())),
    };
    validate_username(&hello.name)?;
    let profile = OfflineProfile {
        uuid: offline_uuid(&hello.name),
        name: hello.name,
    };
    let session_id = random_session_uuid();
    connection
        .send(
            &login::Clientbound::LoginFinished(login::LoginFinished {
                profile: GameProfile {
                    uuid: profile.uuid,
                    name: profile.name.clone(),
                    properties: Vec::<Property>::new(),
                },
                session: session_id,
            })
            .encode(),
        )
        .map_err(|error| format!("send login finished: {error}"))?;

    let (id, payload) = connection.recv().map_err(|error| error.to_string())?;
    match login::Serverbound::decode(id, &payload)
        .map_err(|error| format!("decode LOGIN acknowledgement: {error}"))?
    {
        login::Serverbound::LoginAcknowledged(_) => {}
        packet => {
            return Err(format!(
                "expected login acknowledgement, got {}",
                packet.name()
            ));
        }
    }

    let config_packets = registries.encode_configuration_packets();
    for packet in &config_packets[..2] {
        connection
            .send(packet)
            .map_err(|error| format!("send configuration preamble: {error}"))?;
    }
    loop {
        let (id, payload) = connection.recv().map_err(|error| error.to_string())?;
        match configuration::Serverbound::decode(id, &payload)
            .map_err(|error| format!("decode CONFIGURATION packet: {error}"))?
        {
            configuration::Serverbound::SelectKnownPacks(response) => {
                if !response.packs.is_empty() {
                    return Err("client selected a known pack that was not requested".into());
                }
                break;
            }
            configuration::Serverbound::ClientInformation(_) => {}
            configuration::Serverbound::CustomPayload(_)
            | configuration::Serverbound::CookieResponse(_) => {}
            configuration::Serverbound::KeepAlive(ping) => connection
                .send(&configuration::Clientbound::KeepAlive(ping).encode())
                .map_err(|error| format!("reply to CONFIGURATION keepalive: {error}"))?,
            packet => {
                return Err(format!(
                    "expected known-pack response during CONFIGURATION, got {}",
                    packet.name()
                ));
            }
        }
    }

    for packet in &config_packets[2..] {
        connection
            .send(packet)
            .map_err(|error| format!("send synchronized registries: {error}"))?;
    }
    let mut client_information = None;
    for _ in 0..MAX_CONFIGURATION_PACKETS {
        let (id, payload) = connection.recv().map_err(|error| error.to_string())?;
        match configuration::Serverbound::decode(id, &payload)
            .map_err(|error| format!("decode CONFIGURATION completion: {error}"))?
        {
            configuration::Serverbound::FinishConfiguration(_) => {
                return Ok(ConfiguredConnection {
                    connection,
                    profile,
                    client_information,
                });
            }
            configuration::Serverbound::ClientInformation(info) => {
                client_information = Some(info);
            }
            configuration::Serverbound::ResourcePack(_) => {}
            configuration::Serverbound::CustomPayload(_)
            | configuration::Serverbound::CookieResponse(_) => {}
            configuration::Serverbound::KeepAlive(ping) => connection
                .send(&configuration::Clientbound::KeepAlive(ping).encode())
                .map_err(|error| format!("reply to CONFIGURATION keepalive: {error}"))?,
            packet => {
                return Err(format!(
                    "unsupported CONFIGURATION packet before finish: {}",
                    packet.name()
                ));
            }
        }
    }
    Err("too many CONFIGURATION packets before client finished".into())
}

fn validate_username(name: &str) -> Result<(), String> {
    if !(3..=16).contains(&name.len())
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err("offline username must be 3–16 ASCII letters, digits or underscores".into());
    }
    Ok(())
}

fn random_session_uuid() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let ticks = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let entropy = NEXT.fetch_add(1, Ordering::Relaxed) as u128;
    let mut uuid = ticks ^ (entropy << 64);
    uuid = (uuid & !(0xFu128 << 76)) | (4u128 << 76);
    (uuid & !(0x3u128 << 62)) | (0x2u128 << 62)
}

#[cfg(test)]
mod tests {
    use super::{OfflineProfile, login_and_configure};
    use crate::registry_sync::{RegistryBundle, SYNCHRONIZED_REGISTRIES};
    use horizon_protocol::{
        connection::{Connection, Pipe},
        packets::{
            Empty,
            configuration::{self, RegistryData, RegistryEntry},
            handshake::{self, Intention},
            login,
        },
    };
    use horizon_world::nbt::Tag;

    fn fixture_bundle() -> RegistryBundle {
        RegistryBundle::from_test_parts(
            SYNCHRONIZED_REGISTRIES
                .iter()
                .map(|registry| RegistryData {
                    registry: (*registry).into(),
                    entries: vec![RegistryEntry {
                        id: "minecraft:test".into(),
                        data: Some(Tag::compound([("fixture", Tag::Byte(1))])),
                    }],
                })
                .collect(),
            Vec::new(),
        )
    }

    fn packet(connection: &mut Connection) -> (i32, Vec<u8>) {
        connection.recv().unwrap()
    }

    #[test]
    fn offline_login_and_configuration_complete_over_a_tcp_style_connection() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let bundle = fixture_bundle();
        let server = std::thread::spawn(move || {
            login_and_configure(Connection::local(server_pipe), &bundle).unwrap()
        });
        let mut client = Connection::local(client_pipe);
        client
            .send(
                &handshake::Serverbound::Intention(Intention {
                    protocol: 777,
                    host: "localhost".into(),
                    port: 25565,
                    intent: handshake::LOGIN,
                })
                .encode(),
            )
            .unwrap();
        client
            .send(
                &login::Serverbound::Hello(login::ServerHello {
                    name: "TestPlayer".into(),
                    uuid: 0,
                })
                .encode(),
            )
            .unwrap();
        let (id, payload) = packet(&mut client);
        assert!(matches!(
            login::Clientbound::decode(id, &payload).unwrap(),
            login::Clientbound::LoginFinished(_)
        ));
        client
            .send(&login::Serverbound::LoginAcknowledged(Empty).encode())
            .unwrap();

        let (id, payload) = packet(&mut client);
        assert!(matches!(
            configuration::Clientbound::decode(id, &payload).unwrap(),
            configuration::Clientbound::UpdateEnabledFeatures(_)
        ));
        let (id, payload) = packet(&mut client);
        assert!(matches!(
            configuration::Clientbound::decode(id, &payload).unwrap(),
            configuration::Clientbound::SelectKnownPacks(_)
        ));
        client
            .send(
                &configuration::Serverbound::CustomPayload(configuration::CustomPayload {
                    channel: "minecraft:brand".into(),
                    data: b"vanilla".to_vec(),
                })
                .encode(),
            )
            .unwrap();
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
            let (id, payload) = packet(&mut client);
            match configuration::Clientbound::decode(id, &payload).unwrap() {
                configuration::Clientbound::RegistryData(data) => {
                    seen_registries.push(data.registry);
                }
                configuration::Clientbound::UpdateTags(_) => {}
                configuration::Clientbound::FinishConfiguration(_) => break,
                other => panic!("unexpected configuration packet {other:?}"),
            }
        }
        assert_eq!(seen_registries, SYNCHRONIZED_REGISTRIES);
        client
            .send(&configuration::Serverbound::FinishConfiguration(Empty).encode())
            .unwrap();

        let configured = server.join().unwrap();
        assert_eq!(
            configured.profile,
            OfflineProfile {
                name: "TestPlayer".into(),
                uuid: horizon_protocol::crypto::offline_uuid("TestPlayer"),
            }
        );
        assert!(configured.client_information.is_none());
    }

    #[test]
    fn rejects_invalid_offline_names_before_completing_login() {
        let (server_pipe, client_pipe) = Pipe::pair();
        let bundle = fixture_bundle();
        let server = std::thread::spawn(move || {
            login_and_configure(Connection::local(server_pipe), &bundle)
        });
        let mut client = Connection::local(client_pipe);
        client
            .send(
                &handshake::Serverbound::Intention(Intention {
                    protocol: 777,
                    host: "localhost".into(),
                    port: 25565,
                    intent: handshake::LOGIN,
                })
                .encode(),
            )
            .unwrap();
        client
            .send(
                &login::Serverbound::Hello(login::ServerHello {
                    name: "bad name".into(),
                    uuid: 0,
                })
                .encode(),
            )
            .unwrap();
        let result = server.join().unwrap();
        assert!(result.err().unwrap().contains("offline username"));
    }
}
