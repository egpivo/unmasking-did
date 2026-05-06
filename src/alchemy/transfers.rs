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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_transfers_accepts_string_and_numeric_value() {
        let v = json!([
            {
                "from": "0xAa",
                "to": "0xBb",
                "value": "123",
                "blockNum": "0x10",
                "hash": "0xABCD",
                "asset": "ARB"
            },
            {
                "from": "0xCc",
                "to": "0xDd",
                "value": 7,
                "blockNum": "0x11",
                "hash": "0xEF01",
                "asset": "ETH"
            }
        ]);
        let out = parse_transfers(&v).expect("parse");
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].from_addr, "0xaa");
        assert_eq!(out[0].to_addr, "0xbb");
        assert_eq!(out[0].value.as_deref(), Some("123"));
        assert_eq!(out[0].block_num, Some(16));
        assert_eq!(out[0].tx_hash.as_deref(), Some("0xabcd"));
        assert_eq!(out[1].value.as_deref(), Some("7"));
    }

    #[test]
    fn parse_transfers_rejects_non_array() {
        let err = parse_transfers(&json!({"not":"array"})).unwrap_err();
        assert!(err.to_string().contains("expected `transfers` to be an array"));
    }

    #[test]
    fn parse_one_requires_from_and_to() {
        let err = parse_transfers(&json!([{"to":"0x1"}])).unwrap_err();
        assert!(err.to_string().contains("transfer missing `from`"));

        let err = parse_transfers(&json!([{"from":"0x1"}])).unwrap_err();
        assert!(err.to_string().contains("transfer missing `to`"));
    }

    #[test]
    fn parse_one_handles_invalid_hex_block_as_none() {
        let out = parse_transfers(&json!([{
            "from":"0xAa","to":"0xBb","blockNum":"not_hex"
        }]))
        .expect("parse");
        assert_eq!(out[0].block_num, None);
    }
}
