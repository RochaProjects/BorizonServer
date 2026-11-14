use horizon_server_plugin_api::{HostApiV3, result as transaction_result};
use std::{
    io::Read,
    net::{IpAddr, ToSocketAddrs},
    time::Duration,
};

/// Downloads a bounded public HTTP resource, validating every redirect and DNS result.
pub fn get_public_http_bytes(raw_url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    fn public(ip: IpAddr) -> bool {
        match ip {
            IpAddr::V4(v) => {
                let o = v.octets();
                !v.is_private()
                    && !v.is_loopback()
                    && !v.is_link_local()
                    && !v.is_unspecified()
                    && !v.is_multicast()
                    && !v.is_broadcast()
                    && !(o[0] == 100 && (64..=127).contains(&o[1]))
                    && !(o[0] == 192 && o[1] == 0 && o[2] == 0)
                    && !(o[0] == 192 && o[1] == 0 && o[2] == 2)
                    && !(o[0] == 198 && (o[1] == 18 || o[1] == 19))
                    && !(o[0] == 198 && o[1] == 51 && o[2] == 100)
                    && !(o[0] == 203 && o[1] == 0 && o[2] == 113)
                    && o[0] < 240
            }
            IpAddr::V6(v) => {
                let s = v.segments();
                if let Some(mapped) = v.to_ipv4_mapped() {
                    public(IpAddr::V4(mapped))
                } else {
                    (s[0] & 0xe000) == 0x2000 && !(s[0] == 0x2001 && s[1] == 0x0db8)
                }
            }
        }
    }
    fn validate(url: &url::Url) -> Result<(), String> {
        if !matches!(url.scheme(), "http" | "https")
            || url.username() != ""
            || url.password().is_some()
        {
            return Err("URL precisa ser HTTP/HTTPS público e não pode conter credenciais.".into());
        }
        let host = url.host_str().ok_or("URL sem host.")?;
        let port = url.port_or_known_default().ok_or("Porta URL inválida.")?;
        let addrs: Vec<IpAddr> = if let Ok(ip) = host.parse() {
            vec![ip]
        } else {
            (host, port)
                .to_socket_addrs()
                .map_err(|e| format!("Não foi possível resolver o host: {e}"))?
                .map(|a| a.ip())
                .collect()
        };
        if addrs.is_empty() || addrs.iter().any(|ip| !public(*ip)) {
            return Err("URL bloqueada: o host resolve para uma rede privada ou local.".into());
        }
        Ok(())
    }
    let mut current = url::Url::parse(raw_url).map_err(|e| format!("URL inválida: {e}"))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(60))
        .build();
    for redirect in 0..=5 {
        validate(&current)?;
        let response = match agent
            .get(current.as_str())
            .set("User-Agent", "AuraEssentials-Horizon/1")
            .call()
        {
            Ok(response) => response,
            Err(ureq::Error::Status(code, response)) if (300..400).contains(&code) => {
                if redirect == 5 {
                    return Err("URL excedeu o limite de redirecionamentos.".into());
                }
                let location = response
                    .header("Location")
                    .ok_or("Redirecionamento HTTP sem Location.")?;
                current = current
                    .join(location)
                    .map_err(|e| format!("Redirecionamento inválido: {e}"))?;
                continue;
            }
            Err(error) => return Err(format!("Download HTTP falhou: {error}")),
        };
        let mut bytes = Vec::new();
        response
            .into_reader()
            .take(max_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|e| format!("Falha lendo resposta HTTP: {e}"))?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!("Download excede o limite de {max_bytes} bytes."));
        }
        return Ok(bytes);
    }
    Err("URL excedeu o limite de redirecionamentos.".into())
}

pub fn call_json(
    host: &HostApiV3,
    player: u128,
    request: serde_json::Value,
) -> Option<serde_json::Value> {
    let callback = host.call_json?;
    let input = request.to_string();
    let (high, low) = ((player >> 64) as u64, player as u64);
    let mut output = vec![0_u8; 65_536];
    let mut output_len = 0_usize;
    // SAFETY: buffers remain valid for the duration of the synchronous host call.
    let status = unsafe {
        callback(
            host.context,
            high,
            low,
            input.as_ptr(),
            input.len(),
            output.as_mut_ptr(),
            output.len(),
            &mut output_len,
        )
    };
    (status == transaction_result::OK && output_len <= output.len())
        .then(|| serde_json::from_slice(&output[..output_len]).ok())
        .flatten()
}

pub fn message(host: &HostApiV3, player: u128, text: &str) {
    if let Some(send) = host.common.send_message {
        let (high, low) = ((player >> 64) as u64, player as u64);
        // SAFETY: text remains valid during this synchronous host call.
        unsafe { send(host.common.context, high, low, text.as_ptr(), text.len()) };
    }
}

/// Splits a command completion request into completed words and the token under the cursor.
/// Keeping the last, partially typed word separate makes every command use the same tab rules.
pub fn completion_input(line: &str) -> (Vec<&str>, &str) {
    let line = line.trim_start().trim_start_matches('/');
    let mut words: Vec<_> = line.split_whitespace().collect();
    if line.chars().last().is_some_and(char::is_whitespace) {
        (words, "")
    } else {
        let current = words.pop().unwrap_or("");
        (words, current)
    }
}

/// Filters, sorts and deduplicates dynamic completions before returning them to Brigadier.
pub fn add_suggestions(
    host: &HostApiV3,
    player: u128,
    current: &str,
    candidates: impl IntoIterator<Item = String>,
) {
    let mut candidates: Vec<_> = candidates
        .into_iter()
        .filter(|value| value.to_lowercase().starts_with(&current.to_lowercase()))
        .collect();
    candidates.sort_unstable();
    candidates.dedup();
    for value in candidates.into_iter().take(1000) {
        let _ = call_json(
            host,
            player,
            serde_json::json!({
                "operation": horizon_server_plugin_api::operation::COMMAND_SUGGESTION_ADD,
                "data": {"value": value}
            }),
        );
    }
}
