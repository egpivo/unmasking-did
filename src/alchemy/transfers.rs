use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transfer {
    pub from_addr: String,
    pub to_addr: String,
    pub value: Option<String>,
    pub block_num: Option<i64>,
    pub tx_hash: Option<String>,
    pub asset: Option<String>,
}

pub fn parse_transfers(value: &Value) -> Result<Vec<Transfer>> {
    let arr = value
        .as_array()
        .ok_or_else(|| anyhow!("expected `transfers` to be an array"))?;
    arr.iter().map(parse_one).collect()
}

fn parse_one(v: &Value) -> Result<Transfer> {
    let from_addr = v
        .get("from")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("transfer missing `from`"))?
        .to_lowercase();
    let to_addr = v
        .get("to")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("transfer missing `to`"))?
        .to_lowercase();
    let value = v.get("value").and_then(|x| match x {
        Value::Number(n) => Some(n.to_string()),
        Value::String(s) => Some(s.clone()),
        _ => None,
    });
    let block_num = v
        .get("blockNum")
        .and_then(Value::as_str)
        .and_then(|s| i64::from_str_radix(s.trim_start_matches("0x"), 16).ok());
    let tx_hash = v.get("hash").and_then(Value::as_str).map(str::to_lowercase);
    let asset = v.get("asset").and_then(Value::as_str).map(str::to_string);

    Ok(Transfer {
        from_addr,
        to_addr,
        value,
        block_num,
        tx_hash,
        asset,
    })
}
