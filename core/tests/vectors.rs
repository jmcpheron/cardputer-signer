//! The Safe hash and the blockies against vectors from outside this code:
//! `vectors/safe_tx.json` (viem, and the Safe transaction service for real-*) and
//! `vectors/blockies.json` (the MicroPython blockies.py under CPython). `vectors/SOURCE.md` says where
//! each came from.
use serde_json::Value;
use signer_core::{blockies, gfx::Rgb, hex, safe_tx};

fn load(name: &str) -> Value {
    let path = format!("{}/../vectors/{}", env!("CARGO_MANIFEST_DIR"), name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn addr(v: &Value) -> safe_tx::Address {
    let mut a = [0u8; 20];
    hex::decode(v.as_str().unwrap(), &mut a).unwrap();
    a
}

fn u256(v: &Value) -> safe_tx::U256 {
    match v {
        Value::String(s) => safe_tx::u256_from_dec(s).unwrap(),
        Value::Number(n) => safe_tx::u256_from_u64(n.as_u64().unwrap()),
        _ => panic!("not a number: {v}"),
    }
}

fn h32(v: &Value) -> [u8; 32] {
    let mut h = [0u8; 32];
    hex::decode(v.as_str().unwrap(), &mut h).unwrap();
    h
}

#[test]
fn safe_tx_hashes() {
    let all = load("safe_tx.json");
    let vectors = all["vectors"].as_array().unwrap();
    assert!(vectors.len() >= 17);
    for v in vectors {
        let t = &v["tx"];
        let data_hex = t["data"].as_str().unwrap_or("0x");
        let mut data = vec![0u8; (data_hex.len() - 2) / 2];
        hex::decode(data_hex, &mut data).unwrap();
        let tx = safe_tx::SafeTx {
            to: addr(&t["to"]),
            value: u256(&t["value"]),
            data: &data,
            operation: t["operation"].as_u64().unwrap() as u8,
            safe_tx_gas: u256(&t["safeTxGas"]),
            base_gas: u256(&t["baseGas"]),
            gas_price: u256(&t["gasPrice"]),
            gas_token: addr(&t["gasToken"]),
            refund_receiver: addr(&t["refundReceiver"]),
            nonce: u256(&t["nonce"]),
        };
        let chain = v["chainId"].as_u64().unwrap();
        let safe = addr(&v["safe"]);
        let name = v["name"].as_str().unwrap();
        assert_eq!(safe_tx::domain_separator(chain, &safe), h32(&v["domainSeparator"]), "{name}: domain");
        assert_eq!(safe_tx::safe_tx_hash(chain, &safe, &tx).unwrap(), h32(&v["safeTxHash"]), "{name}: safeTxHash");
    }
}

#[test]
fn verify_code_is_the_last_eight() {
    let h = h32(&Value::from("0x3c9d457126f0f91e1cfdc0bd834ec36f8cf593832d7ac485a2f00bc29ca4cdd4"));
    assert_eq!(&safe_tx::verify_code(&h), b"9ca4cdd4");
}

#[test]
fn blockies_match_the_python() {
    let all = load("blockies.json");
    for v in all["vectors"].as_array().unwrap() {
        let seed = v["seed"].as_str().unwrap();
        let b = blockies::build(seed.as_bytes());
        let cells: String = b.cells.iter().map(|c| char::from(b'0' + c)).collect();
        assert_eq!(cells, v["cells"].as_str().unwrap(), "{seed}: cells");
        let rgb = |k: &str| {
            let a = v[k].as_array().unwrap();
            Rgb(a[0].as_u64().unwrap() as u8, a[1].as_u64().unwrap() as u8, a[2].as_u64().unwrap() as u8)
        };
        assert_eq!(b.bg, rgb("bg"), "{seed}: bg");
        assert_eq!(b.color, rgb("color"), "{seed}: color");
        assert_eq!(b.spot, rgb("spot"), "{seed}: spot");
    }
}

/// A vector's calldata.
fn data_of(v: &Value) -> Vec<u8> {
    let data_hex = v["tx"]["data"].as_str().unwrap_or("0x");
    let mut data = vec![0u8; (data_hex.len() - 2) / 2];
    hex::decode(data_hex, &mut data).unwrap();
    data
}

/// The transaction from a vector, over its calldata.
fn tx_of<'a>(v: &Value, data: &'a [u8]) -> safe_tx::SafeTx<'a> {
    let t = &v["tx"];
    safe_tx::SafeTx {
        to: addr(&t["to"]),
        value: u256(&t["value"]),
        data,
        operation: t["operation"].as_u64().unwrap() as u8,
        safe_tx_gas: u256(&t["safeTxGas"]),
        base_gas: u256(&t["baseGas"]),
        gas_price: u256(&t["gasPrice"]),
        gas_token: addr(&t["gasToken"]),
        refund_receiver: addr(&t["refundReceiver"]),
        nonce: u256(&t["nonce"]),
    }
}

/// The decoder says what the MicroPython decoder says (each vector's `expect`), for every vector
/// it reads. The others (batches, owner
/// and threshold changes, Uniswap) it reads less finely, and red: checked below.
#[test]
fn decode_matches_the_python_decoder() {
    use signer_core::decode::{review, Level};
    let same = ["eth-send-base", "eth-send-base-sepolia", "erc20-transfer", "gas-refund", "big-nonce", "unknown-call",
                "unlimited-approve", "delegatecall-unknown", "rejection", "uniswap-approve", "sim-send"];
    let all = load("safe_tx.json");
    let mut checked = 0;
    for v in all["vectors"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let Some(expect) = v.get("expect") else { continue };
        let data = data_of(v);
        let tx = tx_of(v, &data);
        let r = review(v["chainId"].as_u64().unwrap(), &addr(&v["safe"]), &tx);
        let level = |s: &str| match s { "ok" => Level::Ok, "warn" => Level::Warn, _ => Level::Red };
        if same.contains(&name) {
            assert_eq!(r.summary(), expect["summary"].as_str().unwrap(), "{name}: summary");
            let want: Vec<(String, Level)> = expect["items"].as_array().unwrap().iter()
                .map(|i| (i[0].as_str().unwrap().to_string(), level(i[1].as_str().unwrap()))).collect();
            let got: Vec<(String, Level)> = r.items.iter().flatten().map(|i| (i.title.as_str().to_string(), i.level)).collect();
            assert_eq!(got, want, "{name}: items");
        } else {
            // read less finely than the Python decoder, never more kindly
            let worst = expect["items"].as_array().unwrap().iter().map(|i| level(i[1].as_str().unwrap())).max().unwrap();
            assert!(r.level >= worst, "{name}: {:?} is kinder than the Python decoder's {:?}", r.level, worst);
            assert_eq!(r.level, Level::Red, "{name}: what this can't read is red");
        }
        checked += 1;
    }
    assert_eq!(checked, 15);
}

/// Every page of every vector draws, through the same line the computer sends. An item with an
/// amount and a note gets a page for each.
#[test]
fn every_page_draws() {
    use signer_core::gfx::Frame;
    use signer_core::screens::{self, Page, Pending, H, W};
    let all = load("safe_tx.json");
    let mut buf = vec![0u8; (W * H * 2) as usize];
    for v in all["vectors"].as_array().unwrap() {
        let data = data_of(v);
        let tx = tx_of(v, &data);
        let mut d = [0u8; 2048];
        d[..data.len()].copy_from_slice(&data);
        let fields = signer_core::proto::TxFields {
            chain: v["chainId"].as_u64().unwrap(), safe: addr(&v["safe"]), to: tx.to, value: tx.value,
            data_len: data.len(), operation: tx.operation, safe_tx_gas: tx.safe_tx_gas, base_gas: tx.base_gas,
            gas_price: tx.gas_price, gas_token: tx.gas_token, refund_receiver: tx.refund_receiver, nonce: tx.nonce,
            expect: Some(h32(&v["safeTxHash"])),
        };
        let p = Pending::new(fields, &d).unwrap();
        assert_eq!(p.matches(), Some(true), "{}", v["name"]);
        assert_eq!(p.page(p.pages() - 1), Page::Hash);
        assert_eq!(p.page(p.pages() - 2), Page::Fields);
        for page in 0..p.pages() {
            screens::review(&mut Frame::new(&mut buf, W, H), &p, &d, page);
        }
        if v["name"] == "unlimited-approve" {
            assert_eq!((p.page(0), p.page(1)), (Page::Item(0), Page::Note(0)));
        }
    }
}
