//! Draws the screens on the laptop, exactly as the device draws them, to PNG at 3x:
//!   cargo run --example screens [outdir]      (default target/screens)
//!   cargo run --example screens outdir all  (every page of every vector, for web/)
//! The transactions are the test vectors (`vectors/safe_tx.json`), sent through the same
//! line protocol the device reads.
use serde_json::Value;
use signer_core::gfx::Frame;
use signer_core::proto::{self, Cmd};
use signer_core::screens::{self, Chip, ChipInfo, Pending, H, W};

#[path = "common/png.rs"]
mod png;
use png::png;

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "target/screens".into());
    std::fs::create_dir_all(&out).unwrap();
    let chip = Chip::Found(ChipInfo {
        serial: [0x01, 0x23, 0xf3, 0xac, 0xfd, 0x2a, 0x82, 0x6b, 0xee],
        revision: [0x00, 0x00, 0x60, 0x02],
        config_locked: true,
        data_locked: false,
        counter0: 23,
        pubkey0: None,
    });
    let mut buf = vec![0u8; (W * H * 2) as usize];
    let mut save = |name: &str, draw: &dyn Fn(&mut Frame)| {
        let mut f = Frame::new(&mut buf, W, H);
        draw(&mut f);
        let path = format!("{out}/{name}.png");
        std::fs::write(&path, png(&f, 3)).unwrap();
        println!("{path}");
    };
    save("idle", &|f| screens::idle(f, &chip, None));
    save("idle-no-chip", &|f| screens::idle(f, &Chip::Missing, Some("Rejected #12: Send 0.01 ETH")));

    let vectors: Value = serde_json::from_str(&std::fs::read_to_string(
        format!("{}/../vectors/safe_tx.json", env!("CARGO_MANIFEST_DIR"))).unwrap()).unwrap();
    let find = |n: &str| vectors["vectors"].as_array().unwrap().iter().find(|v| v["name"] == n).unwrap().clone();
    let picks: Vec<(String, bool)> = if std::env::args().nth(2).as_deref() == Some("all") {
        // `all`, for web/: every page of every vector, and each one's hash page with a wrong claim
        vectors["vectors"].as_array().unwrap().iter()
            .flat_map(|v| [false, true].map(|t| (v["name"].as_str().unwrap().to_string(), t)))
            .collect()
    } else {
        [("eth-send-base-sepolia", false), ("erc20-transfer", false), ("unlimited-approve", false), ("gas-refund", false),
         ("delegatecall-unknown", false), ("big-nonce", false), ("add-owner", false), ("eth-send-base-sepolia", true)]
            .map(|(n, t)| (n.to_string(), t)).to_vec()
    };
    let mut reviews = serde_json::Map::new();
    for (name, tamper) in &picks {
        let (name, tamper) = (name.as_str(), *tamper);
        let line = tx_line(&find(name), tamper);
        let mut data = [0u8; 2048];
        let Ok(Cmd::Tx(t)) = proto::parse(&line, &mut data) else { panic!("{name}: {line}") };
        let p = Pending::new(t, &data).unwrap();
        let tag = if tamper { format!("{name}-tampered") } else { name.to_string() };
        if !tamper {
            // what the device says about it, for web/ (reviews.json)
            let level = |l: signer_core::decode::Level| format!("{l:?}").to_lowercase();
            let items: Vec<Value> = p.review.items.iter().flatten()
                .map(|i| serde_json::json!([i.title.as_str(), level(i.level)])).collect();
            reviews.insert(name.into(), serde_json::json!({
                "summary": p.review.summary(), "level": level(p.review.level), "items": items, "pages": p.pages(),
            }));
        }
        for page in 0..p.pages() {
            if tamper && page + 1 < p.pages() {
                continue;
            }
            save(&format!("{tag}-{}", page + 1), &|f| screens::review(f, &p, &data, page));
        }
        if name == "eth-send-base-sepolia" && !tamper {
            save("rejected", &|f| screens::rejected(f, &p));
        }
    }
    if picks.len() > 8 {
        let path = format!("{out}/reviews.json");
        std::fs::write(&path, serde_json::to_string_pretty(&reviews).unwrap()).unwrap();
        println!("{path}");
    }
}

/// The line host/send.py sends for a vector. `tamper` changes the hash the computer claims.
fn tx_line(v: &Value, tamper: bool) -> String {
    let t = &v["tx"];
    let s = |k: &str| match &t[k] { Value::String(s) => s.clone(), x => x.to_string() };
    let mut expect = v["safeTxHash"].as_str().unwrap().to_string();
    if tamper {
        expect.replace_range(expect.len() - 1.., if expect.ends_with('0') { "1" } else { "0" });
    }
    format!("tx chain={} safe={} to={} value={} data={} op={} nonce={} safetxgas={} basegas={} gasprice={} gastoken={} refundreceiver={} expect={}",
            v["chainId"], v["safe"].as_str().unwrap(), s("to"), s("value"), s("data"), s("operation"), s("nonce"),
            s("safeTxGas"), s("baseGas"), s("gasPrice"), s("gasToken"), s("refundReceiver"), expect)
}
