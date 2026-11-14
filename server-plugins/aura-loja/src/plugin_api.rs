use horizon_server_plugin_api::{HostApiV3, result as transaction_result};

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
