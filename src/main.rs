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

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    fn test_account(client: u16) -> Account {
        Account { client, ..Default::default() }
    }

    #[test]
    fn test_deposit() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        assert_eq!(acc.available, dec!(10.0));
        assert_eq!(acc.total, dec!(10.0));
        assert_eq!(acc.held, dec!(0.0));
    }

    #[test]
    fn test_withdrawal() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.withdrawal(dec!(4.0));
        assert_eq!(acc.available, dec!(6.0));
        assert_eq!(acc.total, dec!(6.0));
    }

    #[test]
    fn test_withdrawal_insufficient() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(2.0));
        acc.withdrawal(dec!(3.0));
        assert_eq!(acc.available, dec!(2.0));
    }

    #[test]
    fn test_dispute_valid() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.dispute(1);
        assert_eq!(acc.available, dec!(0.0));
        assert_eq!(acc.held, dec!(10.0));
    }

    #[test]
    fn test_resolve() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.dispute(1);
        acc.resolve(1);
        assert_eq!(acc.available, dec!(10.0));
        assert_eq!(acc.held, dec!(0.0));
    }

    #[test]
    fn test_chargeback() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.dispute(1);
        acc.chargeback(1);
        assert_eq!(acc.total, dec!(0.0));
        assert_eq!(acc.held, dec!(0.0));
        assert!(acc.locked);
    }

    #[test]
    fn test_locked_account_blocks_deposit() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.dispute(1);
        acc.chargeback(1);
        acc.deposit(2, dec!(10.0));
        assert_eq!(acc.total, dec!(0.0));
    }

    #[test]
    fn test_dispute_nonexistent_tx() {
        let mut acc = test_account(1);
        acc.dispute(99); // No tx inserted
        assert_eq!(acc.available, dec!(0.0));
        assert_eq!(acc.held, dec!(0.0));
    }

    #[test]
    fn test_dispute_on_withdrawal_should_be_ignored() {
        let mut acc = test_account(1);
        acc.deposit(1, dec!(10.0));
        acc.withdrawal(dec!(5.0)); // No tx id stored for withdrawal
        acc.dispute(2); // Attempt to dispute non-existent withdrawal
        assert_eq!(acc.available, dec!(5.0));
        assert_eq!(acc.held, dec!(0.0));
        assert_eq!(acc.total, dec!(5.0));
    }

    #[test]
    fn test_dispute_tx_not_owned_by_client_is_ignored() {
        let mut acc1 = test_account(1);
        let mut acc2 = test_account(2);

        // Only acc1 has tx 100
        acc1.deposit(100, dec!(15.0));

        // acc2 tries to dispute tx 100 (which it doesn't own)
        acc2.dispute(100);

        // Assert acc1 remains unchanged
        assert_eq!(acc1.available, dec!(15.0));
        assert_eq!(acc1.held, dec!(0.0));

        // Assert acc2 remains unchanged
        assert_eq!(acc2.available, dec!(0.0));
        assert_eq!(acc2.held, dec!(0.0));
    }
}
