use std::{collections::HashMap, env, error::Error, fs::File, io};
use serde::{Deserialize, Serialize};
use rust_decimal::Decimal;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TxKind {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

#[derive(Debug, Deserialize)]
struct Transaction {
    #[serde(rename = "type")]
    tx_type: TxKind,
    client: u16,
    tx: u32,
    amount: Option<Decimal>,
}

#[derive(Debug, Serialize, Default)]
struct Account {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,

    #[serde(skip)]
    history: HashMap<u32, (Decimal, bool)>, // (amount, disputed?)
}

impl Account {
    fn deposit(&mut self, tx: u32, amount: Decimal) {
        if self.locked { return; }
        self.available += amount;
        self.total += amount;
        self.history.insert(tx, (amount, false));
    }

    fn withdrawal(&mut self, amount: Decimal) {
        if self.locked || self.available < amount { return; }
        self.available -= amount;
        self.total -= amount;
    }

    fn dispute(&mut self, tx: u32) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if !*disputed {
                self.available -= *amount;
                self.held += *amount;
                *disputed = true;
            }
        }
    }

    fn resolve(&mut self, tx: u32) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if *disputed {
                self.available += *amount;
                self.held -= *amount;
                *disputed = false;
            }
        }
    }

    fn chargeback(&mut self, tx: u32) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if *disputed {
                self.held -= *amount;
                self.total -= *amount;
                self.locked = true;
                *disputed = false;
            }
        }
    }
}

fn process_transactions(path: &str) -> Result<(), Box<dyn Error>> {
    let file = File::open(path)?;
    let mut reader = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(file);
    let mut accounts: HashMap<u16, Account> = HashMap::new();

    for result in reader.deserialize() {
        let record: Transaction = result?;
        let account = accounts.entry(record.client).or_insert(Account {
            client: record.client, ..Default::default()
        });

        match record.tx_type {
            TxKind::Deposit => {
                if let Some(amount) = record.amount {
                    account.deposit(record.tx, amount);
                }
            },
            TxKind::Withdrawal => {
                if let Some(amount) = record.amount {
                    account.withdrawal(amount);
                }
            },
            TxKind::Dispute => account.dispute(record.tx),
            TxKind::Resolve => account.resolve(record.tx),
            TxKind::Chargeback => account.chargeback(record.tx),
        }
    }

    let mut writer = csv::Writer::from_writer(io::stdout());
    writer.write_record(["client", "available", "held", "total", "locked"])?;

    for account in accounts.values() {
        writer.serialize(account)?;
    }

    writer.flush()?;
    Ok(())
}

fn main() {
    if let Some(path) = env::args().nth(1) {
        if let Err(err) = process_transactions(&path) {
            eprintln!("Error processing transactions: {}", err);
        }
    } else {
        eprintln!("Usage: cargo run -- transactions.csv > accounts.csv");
    }
}
