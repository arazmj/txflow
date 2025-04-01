use std::{collections::HashMap, env, error::Error, fs::File, io};
use serde::{Deserialize, Serialize};
use rust_decimal::Decimal;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TxType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, Default)]
pub struct ClientId(u32);
#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct TxId(pub u32);

#[derive(Debug, Deserialize)]
struct Transaction {
    #[serde(rename = "type")]
    tx_type: TxType,
    client: ClientId,
    tx: TxId,
    amount: Option<Decimal>,
}

#[derive(Debug, Serialize, Default)]
struct Account {
    client: ClientId,
    available: Decimal,
    held: Decimal,
    locked: bool,

    #[serde(skip)]
    history: HashMap<TxId, (Decimal, bool)>, // (amount, disputed?)
}

impl Account {
    fn deposit(&mut self, tx: TxId, amount: Decimal) {
        if self.locked { return; }
        self.available += amount;
        self.history.insert(tx, (amount, false));
    }

    fn withdrawal(&mut self, amount: Decimal) {
        if self.locked || self.available < amount { return; }
        self.available -= amount;
    }

    fn dispute(&mut self, tx: TxId) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if !*disputed && self.available >= *amount {
                self.available -= *amount;
                self.held += *amount;
                *disputed = true;
            }
        }
    }

    fn resolve(&mut self, tx: TxId) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if *disputed {
                self.available += *amount;
                self.held -= *amount;
                *disputed = false;
            }
        }
    }

    fn chargeback(&mut self, tx: TxId) {
        if self.locked { return; }
        if let Some((amount, disputed)) = self.history.get_mut(&tx) {
            if *disputed {
                self.held -= *amount;
                self.locked = true;
                *disputed = false;
            }
        }
    }
}

fn process_transactions(path: &str) -> Result<(), Box<dyn Error>> {
    let file = File::open(path)?;
    let mut reader = csv::ReaderBuilder::new().trim(csv::Trim::All).from_reader(file);
    let mut accounts: HashMap<ClientId, Account> = HashMap::new();

    for result in reader.deserialize() {
        let record: Transaction = result?;
        let account = accounts.entry(record.client).or_insert(Account {
            client: record.client, ..Default::default()
        });

        match record.tx_type {
            TxType::Deposit => {
                if let Some(amount) = record.amount {
                    account.deposit(record.tx, amount);
                }
            },
            TxType::Withdrawal => {
                if let Some(amount) = record.amount {
                    account.withdrawal(amount);
                }
            },
            TxType::Dispute => account.dispute(record.tx),
            TxType::Resolve => account.resolve(record.tx),
            TxType::Chargeback => account.chargeback(record.tx),
        }
    }

    let mut writer = csv::Writer::from_writer(io::stdout());
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

    fn test_account(client: ClientId) -> Account {
        Account { client, ..Default::default() }
    }

    #[test]
    fn test_deposit() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        assert_eq!(account.available, dec!(10.0));
        assert_eq!(account.held, dec!(0.0));
    }

    #[test]
    fn test_withdrawal() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.withdrawal(dec!(4.0));
        assert_eq!(account.available, dec!(6.0));
    }

    #[test]
    fn test_withdrawal_insufficient() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(2.0));
        account.withdrawal(dec!(3.0));
        assert_eq!(account.available, dec!(2.0));
    }

    #[test]
    fn test_dispute_valid() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.dispute(TxId(1));
        assert_eq!(account.available, dec!(0.0));
        assert_eq!(account.held, dec!(10.0));
    }

    #[test]
    fn test_resolve() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.dispute(TxId(1));
        account.resolve(TxId(1));
        assert_eq!(account.available, dec!(10.0));
        assert_eq!(account.held, dec!(0.0));
    }

    #[test]
    fn test_chargeback() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.dispute(TxId(1));
        account.chargeback(TxId(1));
        assert_eq!(account.held, dec!(0.0));
        assert!(account.locked);
    }

    #[test]
    fn test_locked_account_blocks_deposit() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.dispute(TxId(1));
        account.chargeback(TxId(1));
        account.deposit(TxId(2), dec!(10.0));
        assert_eq!(account.available, dec!(0.0));
    }

    #[test]
    fn test_dispute_nonexistent_tx() {
        let mut account = test_account(ClientId(1));
        account.dispute(TxId(99)); // No tx inserted
        assert_eq!(account.available, dec!(0.0));
        assert_eq!(account.held, dec!(0.0));
    }

    #[test]
    fn test_dispute_on_withdrawal_should_be_ignored() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.withdrawal(dec!(5.0)); // No tx id stored for withdrawal
        account.dispute(TxId(2)); // Attempt to dispute non-existent withdrawal
        assert_eq!(account.available, dec!(5.0));
        assert_eq!(account.held, dec!(0.0));
    }

    #[test]
    fn test_dispute_tx_not_owned_by_client_is_ignored() {
        let mut acc1 = test_account(ClientId(1));
        let mut acc2 = test_account(ClientId(2));

        // Only acc1 has tx 100
        acc1.deposit(TxId(100), dec!(15.0));

        // acc2 tries to dispute tx 100 (which it doesn't own)
        acc2.dispute(TxId(100));

        // Assert acc1 remains unchanged
        assert_eq!(acc1.available, dec!(15.0));
        assert_eq!(acc1.held, dec!(0.0));

        // Assert acc2 remains unchanged
        assert_eq!(acc2.available, dec!(0.0));
        assert_eq!(acc2.held, dec!(0.0));
    }

    #[test]
    fn test_dispute_after_funds_already_withdrawn_should_fail() {
        let mut account = test_account(ClientId(1));
        account.deposit(TxId(1), dec!(10.0));
        account.withdrawal(dec!(10.0));
        account.dispute(TxId(1)); // Should be ignored
        assert_eq!(account.available, dec!(0.0));
        assert_eq!(account.held, dec!(0.0));
    }

}
