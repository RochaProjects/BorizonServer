//! Finding a player's account by name, as an online-mode server does for the
//! names given to `op`, `whitelist` and `ban` (CachedUserNameToIdResolver,
//! MinecraftServicesProfileRepository.findProfileByName): the services'
//! `profiles` / `getByName` endpoint, found through the discovery document.
use crate::{
    json::{self, Json},
    user_lists::NameAndId,
};
use horizon_protocol::{packets::login::Property, rsa::RsaPublicKey};
use std::time::Duration;

/// Mojang's services discovery document.
pub const DEFAULT_DISCOVERY: &str = "https://discovery.minecraftservices.com/minecraft/client";

/// The discovery document's URL: `HORIZON_API_DISCOVERY` overrides it like
/// authlib's `-Dminecraft.api.discovery.host`.
pub fn discovery_url() -> String {
    std::env::var("HORIZON_API_DISCOVERY")
        .ok()
        .filter(|u| !u.is_empty())
        .unwrap_or_else(|| DEFAULT_DISCOVERY.to_owned())
}

/// StringUtil.isValidPlayerName: at most 16 UTF-16 units, none a control
/// character, a space or above ASCII.
pub fn valid_player_name(name: &str) -> bool {
    name.encode_utf16().count() <= 16 && name.encode_utf16().all(|c| c > 32 && c < 127)
}

/// How a server resolves a name it was given (CachedUserNameToIdResolver.get,
/// without the usercache.json cache): an offline server makes the offline
/// profile; an online one asks the services (for the name in lower case) and
/// knows nobody they don't.
pub fn resolve(name: &str, online: bool) -> Option<NameAndId> {
    if online && valid_player_name(name) {
        find_by_name(&discovery_url(), &name.to_ascii_lowercase())
    } else {
        (!online).then(|| NameAndId::offline(name))
    }
}

/// MinecraftServicesProfileRepository.findProfileByName: the account named
/// `name` (any case), or None when there is none or the services don't
/// answer (vanilla logs both as "Couldn't find profile with name").
pub fn find_by_name(discovery: &str, name: &str) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url = endpoint(&doc, "profiles", "getByName")?.replace("{name}", &name.to_lowercase());
    found(&agent.get(&url).call().ok()?.into_string().ok()?, name)
}

/// YggdrasilMinecraftSessionService.fetchProfile(id, true), as the management protocol finds a player given by
/// id that the user cache doesn't know: the `session` / `getProfileById` endpoint, or None.
pub fn find_by_id(discovery: &str, id: u128) -> Option<NameAndId> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let doc = json::parse(&agent.get(discovery).call().ok()?.into_string().ok()?, true).ok()?;
    let url =
        endpoint(&doc, "session", "getProfileById")?.replace("{profileId}", &format!("{id:032x}"));
    let separator = if url.contains('?') { '&' } else { '?' };
    let answer = agent
        .get(&format!("{url}{separator}unsigned=false"))
        .call()
        .ok()?
        .into_string()
        .ok()?;
    let name = json::parse(&answer, true).ok()?.get("name")?.as_string()?;
    found(&answer, &name).filter(|user| user.id == id)
}

/// A profile verified by Minecraft's session service after an online-mode
/// encryption challenge.
#[derive(Clone, Debug, PartialEq)]
pub struct JoinedProfile {
    pub id: u128,
    pub name: String,
    pub properties: Vec<Property>,
}

enum VerificationError {
    Unavailable(String),
    InvalidResponse(String),
}

/// MinecraftServicesSessionService.hasJoinedServer. A missing profile means
/// the client did not authenticate this username for this server hash; an
/// error means the service could not be reached or its response was invalid.
pub fn has_joined(
    name: &str,
    server_hash: &str,
    ip: Option<std::net::IpAddr>,
) -> Result<Option<JoinedProfile>, String> {
    match has_joined_once(name, server_hash, ip) {
        Err(VerificationError::Unavailable(_)) => {
            std::thread::sleep(Duration::from_millis(500));
            has_joined_once(name, server_hash, ip).map_err(VerificationError::message)
        }
        Ok(profile) => Ok(profile),
        Err(error) => Err(error.message()),
    }
}

fn has_joined_once(
    name: &str,
    server_hash: &str,
    ip: Option<std::net::IpAddr>,
) -> Result<Option<JoinedProfile>, VerificationError> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let discovery = json::parse(
        &agent
            .get(&discovery_url())
            .call()
            .map_err(|error| {
                VerificationError::Unavailable(format!(
                    "Minecraft Services discovery request failed: {error}"
                ))
            })?
            .into_string()
            .map_err(|error| {
                VerificationError::Unavailable(format!(
                    "read Minecraft Services discovery response: {error}"
                ))
            })?,
        true,
    )
    .map_err(|error| {
        VerificationError::InvalidResponse(format!(
            "parse Minecraft Services discovery response: {error}"
        ))
    })?;
    let endpoint = endpoint(&discovery, "session", "verify").ok_or_else(|| {
        VerificationError::InvalidResponse(
            "Minecraft Services discovery has no session verification endpoint".into(),
        )
    })?;
    let mut query = format!(
        "username={}&serverId={}",
        url_encode(name),
        url_encode(server_hash)
    );
    if let Some(ip) = ip {
        query.push_str(&format!("&ip={}", url_encode(&ip.to_string())));
    }
    let separator = if endpoint.contains('?') { '&' } else { '?' };
    let response = match agent.get(&format!("{endpoint}{separator}{query}")).call() {
        Ok(response) => response,
        Err(ureq::Error::Status(204, _)) => return Ok(None),
        Err(ureq::Error::Status(status, response)) if matches!(status, 401 | 403) => {
            let _ = response.into_string();
            return Ok(None);
        }
        Err(ureq::Error::Status(status, response)) if status < 500 => {
            let body = response.into_string().unwrap_or_default();
            return Err(VerificationError::InvalidResponse(format!(
                "Minecraft session verification returned HTTP {status}: {body}"
            )));
        }
        Err(error) => {
            return Err(VerificationError::Unavailable(format!(
                "Minecraft session verification failed: {error}"
            )));
        }
    };
    let body = response.into_string().map_err(|error| {
        VerificationError::Unavailable(format!(
            "read Minecraft session verification response: {error}"
        ))
    })?;
    if body.trim().is_empty() {
        return Ok(None);
    }
    let profile = json::parse(&body, true).map_err(|error| {
        VerificationError::InvalidResponse(format!("parse Minecraft session profile: {error}"))
    })?;
    let Some(id) = profile
        .get("id")
        .and_then(Json::as_string)
        .map(|id| id.replace('-', ""))
        .and_then(|id| {
            (id.len() == 32)
                .then(|| u128::from_str_radix(&id, 16).ok())
                .flatten()
        })
    else {
        return Ok(None);
    };
    let Some(player_name) = profile.get("name").and_then(Json::as_string) else {
        return Ok(None);
    };
    if !player_name.eq_ignore_ascii_case(name) {
        return Ok(None);
    }
    let properties = match profile.get("properties") {
        Some(Json::Array(properties)) => properties
            .iter()
            .filter_map(|property| {
                Some(Property {
                    name: property.get("name")?.as_string()?,
                    value: property.get("value")?.as_string()?,
                    signature: property.get("signature").and_then(Json::as_string),
                })
            })
            .collect(),
        _ => Vec::new(),
    };
    Ok(Some(JoinedProfile {
        id,
        name: player_name,
        properties,
    }))
}

impl VerificationError {
    fn message(self) -> String {
        match self {
            Self::Unavailable(message) | Self::InvalidResponse(message) => message,
        }
    }
}

fn url_encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'-' | b'*' | b'_' => {
                encoded.push(char::from(byte));
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

/// MinecraftServicesKeyInfo.get: the services' public keys from the
/// discovery document's `authentication` / `getPublicKeys` endpoint. The
/// profile-key keys (`playerCertificateKeys`, ServicesKeyType.PROFILE_KEY)
/// sign the players' chat keys; secure chat checks them. The error is the
/// line vanilla logs, after which it has no keys.
pub fn fetch_profile_key_keys(discovery: &str) -> Result<Vec<RsaPublicKey>, &'static str> {
    const FAILED: &str = "Failed to request Minecraft Services public key";
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .user_agent(&format!("Horizon/{}", horizon_core::release::VERSION))
        .build();
    let get = |url: &str| agent.get(url).call().ok()?.into_string().ok();
    let doc = get(discovery)
        .and_then(|d| json::parse(&d, true).ok())
        .ok_or(FAILED)?;
    let url = endpoint(&doc, "authentication", "getPublicKeys").ok_or(FAILED)?;
    profile_key_keys(&get(&url).ok_or(FAILED)?)
}

/// The key set answer: `{"profilePropertyKeys": [...], "playerCertificateKeys":
/// [...]}`, each `{"publicKey": base64 X.509}`. One bad key makes the whole
/// answer malformed (parseList throws).
pub fn profile_key_keys(answer: &str) -> Result<Vec<RsaPublicKey>, &'static str> {
    use base64::Engine;
    const MALFORMED: &str = "Received malformed Minecraft Services public key data";
    let doc = json::parse(answer, true).map_err(|_| MALFORMED)?;
    let list = |name: &str| -> Result<Vec<RsaPublicKey>, &'static str> {
        let Some(entries) = doc.get(name) else {
            return Ok(Vec::new());
        };
        let Json::Array(entries) = entries else {
            return Err(MALFORMED);
        };
        entries
            .iter()
            .map(|e| {
                let text = e
                    .get("publicKey")
                    .and_then(Json::as_string)
                    .ok_or(MALFORMED)?;
                let der = base64::engine::general_purpose::STANDARD
                    .decode(text)
                    .map_err(|_| MALFORMED)?;
                RsaPublicKey::from_der(&der).map_err(|_| MALFORMED)
            })
            .collect()
    };
    list("profilePropertyKeys")?;
    list("playerCertificateKeys")
}

/// MinecraftServicesDiscoveryService.getUrl(service, endpoint).
fn endpoint(doc: &Json, service: &str, name: &str) -> Option<String> {
    doc.get("discovery")?
        .get(service)?
        .get("endpoints")?
        .get(name)?
        .get("uri")?
        .as_string()
}

/// The answer, authlib's NameAndId: `{"id", "name"}`, the id undashed.
pub fn found(answer: &str, name: &str) -> Option<NameAndId> {
    let profile = json::parse(answer, true).ok()?;
    let found = profile.get("name")?.as_string()?;
    let id = profile.get("id")?.as_string()?.replace('-', "");
    (found.eq_ignore_ascii_case(name) && id.len() == 32).then_some(())?;
    Some(NameAndId {
        id: u128::from_str_radix(&id, 16).ok()?,
        name: found,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_answers() {
        assert!(valid_player_name("Alex_123"));
        assert!(!valid_player_name("two words"));
        assert!(!valid_player_name("ThisNameIsTooLong"));
        assert!(!valid_player_name("caf\u{e9}"));
        assert_eq!(resolve("Alex", false), Some(NameAndId::offline("Alex")));
        assert_eq!(resolve("not valid", true), None);
        let answer = r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":"Notch"}"#;
        assert_eq!(
            found(answer, "notch"),
            Some(NameAndId {
                id: 0x069a79f444e94726a5befca90e38aaf5,
                name: "Notch".into()
            })
        );
        assert_eq!(found(answer, "Steve"), None);
        assert_eq!(found("{}", "Notch"), None);
        assert_eq!(found("", "Notch"), None);
    }

    #[test]
    fn services_key_sets() {
        // The 1024-bit test key of horizon-protocol's RSA tests, base64.
        let key = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQCrAmB0+SRQqPcm84lbsRF2yo8GXzEAzUFDusT302dO3P+VtBoFJdI2WlScEG7GaFiBs5IXdCD9Mf9cFE/xG6JDEedRzSxrXRgXtq9mhqQ8ldYSQ2EwBeaUW2CTzZUiPOxeiFaCbanS7hsyM2KsHOvqs2WcvEUxmptUDdqBakeY/wIDAQAB";
        let answer = format!(
            r#"{{"profilePropertyKeys":[{{"publicKey":"{key}"}}],"playerCertificateKeys":[{{"publicKey":"{key}"}},{{"publicKey":"{key}"}}]}}"#
        );
        let keys = profile_key_keys(&answer).unwrap();
        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].size(), 128);
        assert_eq!(profile_key_keys("{}").unwrap().len(), 0);
        let bad = r#"{"profilePropertyKeys":[{"publicKey":"AAAA"}],"playerCertificateKeys":[]}"#;
        assert!(profile_key_keys(bad).is_err());
        assert!(profile_key_keys("not json").is_err());
    }
}
