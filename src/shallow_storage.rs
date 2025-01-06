use std::{cell::RefCell, convert::Infallible, io};

use fuel_core_client::client::FuelClient;
use fuel_core_types::{services::executor::StorageReadReplayEvent, tai64::Tai64};
use fuel_vm::{
    error::{InterpreterError, RuntimeError},
    fuel_storage::{StorageRead, StorageSize, StorageWrite},
    fuel_types::BlockHeight,
    prelude::{Bytes32, StorageAsRef, StorageInspect, StorageMutate},
    storage::{
        BlobData, ContractsAssets, ContractsAssetsStorage, ContractsRawCode, ContractsState,
        ContractsStateData, ContractsStateKey, InterpreterStorage, UploadedBytecodes,
    },
};

fn increment_array<const S: usize>(mut array: [u8; S]) -> [u8; S] {
    let mut carry = 0;
    for i in (0..S).rev() {
        if let Some(value) = array[i].checked_add(1) {
            if let Some(value) = value.checked_add(carry) {
                array[i] = value;
            } else {
                array[i] = 0;
                carry = 1;
            }
        } else {
            array[i] = 0;
            carry = 1;
        }
    }

    if carry == 1 {
        [0; S]
    } else {
        array
    }
}

pub struct ShallowStorage {
    pub block_height: BlockHeight,
    pub timestamp: Tai64,
    pub consensus_parameters_version: u32,
    pub state_transition_version: u32,
    pub coinbase: fuel_vm::prelude::ContractId,
    pub reads: RefCell<Vec<StorageReadReplayEvent>>,
    /// Client used to read data from the node as needed
    pub client: FuelClient,
}

impl ShallowStorage {
    fn pop_read_of_column(&self, column: &str) -> StorageReadReplayEvent {
        println!(">>> IN {column:?}");
        if column == "ContractsAssets" {
            return StorageReadReplayEvent {
                column: "ContractsAssets".to_string(),
                key: vec![],
                value: Some(vec![0, 0, 0, 0, 0, 0, 0, 0]),
            };
        }
        loop {
            let item = self.reads.borrow_mut().remove(0);
            if item.column == column {
                println!("<<< OUT");
                return item;
            }
            println!("Skipping read ({:?} != {:?})", column, item.column);
        }
    }
}

macro_rules! storage_rw {
    ($table:ident, $convert:expr) => {
        impl StorageSize<$table> for ShallowStorage {
            fn size_of_value(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<usize>, Self::Error> {
                println!("size_of_value??? {key:?}");
                let head = self.pop_read_of_column(stringify!($table));
                println!("size_of_value {key:?} (==? {:?})", head.key);
                Ok(head.value.map(|v| v.len()))
            }
        }

        impl StorageInspect<$table> for ShallowStorage {
            type Error = Error;

            fn get(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<std::borrow::Cow<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>>, Self::Error> {
                println!("get??? {key:?}");
                let head = self.pop_read_of_column(stringify!($table));
                println!("get {key:?} (==? {:?})", head.key);
                Ok(head.value.map($convert).map(std::borrow::Cow::Owned))
            }

            fn contains_key(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<bool, Self::Error> {
                println!("contains_key??? {key:?}");
                let head = self.pop_read_of_column(stringify!($table));
                println!("contains_key {key:?} (==? {:?})", head.key);
                Ok(head.value.is_some())
            }
        }

        impl StorageRead<$table> for ShallowStorage {
            fn read(
                &self,
                key: &<$table as fuel_vm::fuel_storage::Mappable>::Key,
                offset: usize,
                buf: &mut [u8],
            ) -> Result<Option<usize>, Self::Error> {
                println!("read??? {key:?}");
                let head = self.pop_read_of_column(stringify!($table));
                println!("read {key:?} {offset:?} (==? {:?}) ({})", head.key, head.value.is_some());
                let Some(value) = head.value else {
                    return Ok(None);
                };
                buf.copy_from_slice(&value[offset..][..buf.len()]);
                Ok(Some(buf.len()))
            }

            fn read_alloc(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<Vec<u8>>, Self::Error> {
                todo!("read_alloc {key:?}")
            }
        }

        impl StorageMutate<$table> for ShallowStorage {
            fn replace(
                &mut self,
                key: &<$table as fuel_vm::fuel_storage::Mappable>::Key,
                value: &<$table as fuel_vm::fuel_storage::Mappable>::Value,
            ) -> Result<Option<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>, Self::Error> {
                println!("replace??? {key:?} (value={value:?})");
                let head = self.pop_read_of_column(stringify!($table));
                println!("replace {key:?} (==? {:?})", head.key);
                Ok(head.value.map($convert))
            }

            fn take(&mut self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>, Self::Error> {
                todo!("take {key:?}")
            }
        }


        impl StorageWrite<$table> for ShallowStorage {
            fn write_bytes(&mut self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key, buf: &[u8]) -> Result<usize, Self::Error> {
                todo!("write_bytes {key:?}")
            }

            fn replace_bytes(
                &mut self,
                key: &<$table as fuel_vm::fuel_storage::Mappable>::Key,
                buf: &[u8],
            ) -> Result<(usize, Option<Vec<u8>>), Self::Error> {
                let head = self.pop_read_of_column(stringify!($table));
                println!("replace_bytes {key:?} (==? {:?})", head.key);
                Ok((buf.len(), head.value))
            }

            fn take_bytes(&mut self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<Vec<u8>>, Self::Error> {
                todo!("take_bytes {key:?}")
            }
        }
    };
}

storage_rw!(ContractsRawCode, |data| todo!(
    "ContractsRawCode from bytes {data:?}"
));
storage_rw!(ContractsState, |data| ContractsStateData(data));
storage_rw!(ContractsAssets, |data| {
    assert_eq!(data.len(), 8);
    let mut buffer = [0u8; 8];
    buffer.copy_from_slice(&data);
    u64::from_be_bytes(buffer)
});
storage_rw!(UploadedBytecodes, |data| todo!(
    "UploadedBytecodes from bytes {data:?}"
));
storage_rw!(BlobData, |data| todo!("BlobData from bytes {data:?}"));

impl ContractsAssetsStorage for ShallowStorage {
    fn contract_asset_id_balance(
        &self,
        id: &fuel_vm::prelude::ContractId,
        asset_id: &fuel_vm::prelude::AssetId,
    ) -> Result<Option<fuel_vm::prelude::Word>, Self::Error> {
        println!("### 1 {id:?} {asset_id:?}");
        let balance = self
            .storage::<fuel_vm::storage::ContractsAssets>()
            .get(&(id, asset_id).into())?
            .map(std::borrow::Cow::into_owned);

        println!("### /1");
        Ok(balance)
    }

    fn contract_asset_id_balance_insert(
        &mut self,
        contract: &fuel_vm::prelude::ContractId,
        asset_id: &fuel_vm::prelude::AssetId,
        value: fuel_vm::prelude::Word,
    ) -> Result<(), Self::Error> {
        println!("### 2 {contract:?} {asset_id:?} {value}");
        fuel_vm::prelude::StorageMutate::<fuel_vm::storage::ContractsAssets>::insert(
            self,
            &(contract, asset_id).into(),
            &value,
        )
    }

    fn contract_asset_id_balance_replace(
        &mut self,
        contract: &fuel_vm::prelude::ContractId,
        asset_id: &fuel_vm::prelude::AssetId,
        value: fuel_vm::prelude::Word,
    ) -> Result<Option<fuel_vm::prelude::Word>, Self::Error> {
        println!("### 3 {contract:?} {asset_id:?} {value}");
        fuel_vm::prelude::StorageMutate::<fuel_vm::storage::ContractsAssets>::replace(
            self,
            &(contract, asset_id).into(),
            &value,
        )
        // Ok(Some(0)) // TODO: this followed by get/insert behaves incorrectly, the replace should be skipped?
    }
}

#[derive(Debug)]
pub enum Error {
    /// Failed to fetch data from the node
    NetworkError(io::Error),
    /// This block couldn't have been included
    InvalidBlock,
}
impl From<Error> for RuntimeError<Error> {
    fn from(e: Error) -> Self {
        RuntimeError::Storage(e)
    }
}
impl From<Error> for InterpreterError<Error> {
    fn from(e: Error) -> Self {
        InterpreterError::Storage(e)
    }
}

impl InterpreterStorage for ShallowStorage {
    type DataError = Error;

    fn block_height(&self) -> Result<BlockHeight, Self::DataError> {
        Ok(self.block_height)
    }

    fn consensus_parameters_version(&self) -> Result<u32, Self::DataError> {
        Ok(self.consensus_parameters_version)
    }

    fn state_transition_version(&self) -> Result<u32, Self::DataError> {
        Ok(self.state_transition_version)
    }

    fn timestamp(
        &self,
        height: fuel_vm::fuel_types::BlockHeight,
    ) -> Result<fuel_vm::prelude::Word, Self::DataError> {
        match height {
            height if height > self.block_height => Err(Error::InvalidBlock),
            height if height == self.block_height => Ok(self.timestamp.0),
            height => tokio::runtime::Handle::current().block_on(async {
                match self.client.block_by_height(height).await {
                    Ok(Some(block)) => Ok(block.header.time.0),
                    Ok(None) => Err(Error::InvalidBlock),
                    Err(e) => Err(Error::NetworkError(e)),
                }
            }),
        }
    }

    fn block_hash(
        &self,
        block_height: fuel_vm::fuel_types::BlockHeight,
    ) -> Result<fuel_vm::prelude::Bytes32, Self::DataError> {
        // Block header hashes for blocks with height greater than or equal to current block height are zero (0x00**32).
        // https://github.com/FuelLabs/fuel-specs/blob/master/specs/vm/instruction_set.md#bhsh-block-hash
        if block_height >= self.block_height || block_height == Default::default() {
            Ok(Bytes32::zeroed())
        } else {
            tokio::runtime::Handle::current().block_on(async {
                match self.client.block_by_height(block_height).await {
                    Ok(Some(block)) => Ok(block.id),
                    Ok(None) => Err(Error::InvalidBlock),
                    Err(e) => Err(Error::NetworkError(e)),
                }
            })
        }
    }

    fn coinbase(&self) -> Result<fuel_vm::prelude::ContractId, Self::DataError> {
        Ok(self.coinbase)
    }

    fn set_consensus_parameters(
        &mut self,
        version: u32,
        consensus_parameters: &fuel_vm::prelude::ConsensusParameters,
    ) -> Result<Option<fuel_vm::prelude::ConsensusParameters>, Self::DataError> {
        unreachable!("Cannot be called by a script");
    }

    fn set_state_transition_bytecode(
        &mut self,
        version: u32,
        hash: &fuel_vm::prelude::Bytes32,
    ) -> Result<Option<fuel_vm::prelude::Bytes32>, Self::DataError> {
        unreachable!("Cannot be called by a script");
    }

    fn contract_state_range(
        &self,
        id: &fuel_vm::prelude::ContractId,
        start_key: &fuel_vm::prelude::Bytes32,
        range: usize,
    ) -> Result<Vec<Option<std::borrow::Cow<fuel_vm::storage::ContractsStateData>>>, Self::DataError>
    {
        println!("contract_state_range {id:?} {start_key:?} {range:?}");
        let mut results = Vec::new();
        let mut key = **start_key;
        for offset in 0..(range as u64) {
            let state_key = ContractsStateKey::new(id, &key.into());
            let value = self
                .storage::<fuel_vm::storage::ContractsState>()
                .get(&state_key)?;
            results.push(value);
            key = increment_array(key);
        }
        Ok(results)
    }

    fn contract_state_insert_range<'a, I>(
        &mut self,
        contract: &fuel_vm::prelude::ContractId,
        start_key: &fuel_vm::prelude::Bytes32,
        values: I,
    ) -> Result<usize, Self::DataError>
    where
        I: Iterator<Item = &'a [u8]>,
    {
        println!("contract_state_insert_range {contract:?} {start_key:?}");
        // We need to return the number of keys that were previously unset
        self.contract_state_range(contract, start_key, values.count())
            .map(|values| values.iter().filter(|v| v.is_none()).count())
    }

    fn contract_state_remove_range(
        &mut self,
        contract: &fuel_vm::prelude::ContractId,
        start_key: &fuel_vm::prelude::Bytes32,
        range: usize,
    ) -> Result<Option<()>, Self::DataError> {
        todo!()
    }
}
