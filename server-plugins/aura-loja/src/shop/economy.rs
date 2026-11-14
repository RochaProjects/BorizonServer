use super::{
    ShopItem,
    ui::{bytes, message},
};
use horizon_server_plugin_api::{HostApiV3, ItemDeltaV2, result as transaction_result};

pub(super) fn transact_shop_item(
    host: &HostApiV3,
    player: u128,
    item: &ShopItem,
    currency: &str,
    quantity: u32,
    buying: bool,
) -> i32 {
    let Some(transact) = host.common.transact else {
        return transaction_result::STORAGE_ERROR;
    };
    let units = if buying { item.buy } else { item.sell };
    if !units.is_finite() || units < 0.0 {
        return transaction_result::INVALID_REQUEST;
    }
    let balance_delta = (units * f64::from(quantity) * 100.0).round();
    if balance_delta > i64::MAX as f64 {
        return transaction_result::INVALID_REQUEST;
    }
    let amount = item.amount.checked_mul(quantity);
    let Some(amount) = amount.and_then(|n| i32::try_from(n).ok()) else {
        return transaction_result::INVALID_REQUEST;
    };
    if amount == 0 {
        return transaction_result::INVALID_REQUEST;
    }
    let material = item.material.to_ascii_lowercase();
    let item_id = format!("minecraft:{material}");
    let (item_ptr, item_len) = bytes(&item_id);
    let (currency_ptr, currency_len) = bytes(currency);
    let delta = if buying {
        -(balance_delta as i64)
    } else {
        balance_delta as i64
    };
    let item_delta = ItemDeltaV2 {
        item: item_ptr,
        item_len,
        delta: if buying { amount } else { -amount },
    };
    let (high, low) = ((player >> 64) as u64, player as u64);
    // SAFETY: the API pointers and item id remain alive for this synchronous host call.
    unsafe {
        transact(
            host.common.context,
            high,
            low,
            currency_ptr,
            currency_len,
            delta,
            &item_delta,
            1,
        )
    }
}

pub(super) fn trade_message(host: &HostApiV3, player: u128, result: i32, buying: bool) {
    let text = match result {
        transaction_result::OK if buying => "Compra concluída.",
        transaction_result::OK => "Venda concluída.",
        transaction_result::INSUFFICIENT_BALANCE => "Saldo insuficiente.",
        transaction_result::ITEMS_REJECTED if buying => {
            "Não há espaço suficiente no inventário para receber os itens."
        }
        transaction_result::ITEMS_REJECTED => "Você não tem os itens necessários no inventário.",
        transaction_result::STORAGE_ERROR => {
            "Falha ao salvar a transação. Confira os logs do servidor."
        }
        _ => "Não foi possível concluir a transação.",
    };
    message(host, player, text);
}

pub(super) fn balance_text(host: &HostApiV3, player: u128, currency: &str) -> Option<String> {
    let get = host.common.get_balance?;
    let (currency_ptr, currency_len) = bytes(currency);
    let (high, low) = ((player >> 64) as u64, player as u64);
    let mut balance = 0_i64;
    // SAFETY: output and currency bytes live through this synchronous host call.
    let result = unsafe {
        get(
            host.common.context,
            high,
            low,
            currency_ptr,
            currency_len,
            &mut balance,
        )
    };
    (result == transaction_result::OK).then(|| price_units(balance))
}

pub(super) fn price_units(units: i64) -> String {
    let sign = if units < 0 { "-" } else { "" };
    let absolute = units.unsigned_abs();
    let whole = absolute / 100;
    let fraction = absolute % 100;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        format!("{sign}{whole},{fraction:02}")
            .trim_end_matches('0')
            .to_owned()
    }
}
