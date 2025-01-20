use anyhow::anyhow;
use fuel_core_client::client::FuelClient;
use fuel_core_types::{services::executor::StorageReadReplayEvent, tai64::Tai64};
use fuel_vm::{
    error::{InterpreterError, RuntimeError},
    fuel_storage::{StorageRead, StorageSize, StorageWrite},
    fuel_types::BlockHeight,
    prelude::*,
    storage::{
        BlobData, ContractsAssetKey, ContractsAssets, ContractsAssetsStorage, ContractsRawCode,
        ContractsState, ContractsStateData, ContractsStateKey, InterpreterStorage,
        UploadedBytecodes,
    },
};
use primitive_types::U256;
use std::{cell::RefCell, collections::HashMap, convert::Infallible, io};

/// Kludgy cache for some reads that work weirdly
#[derive(Default)]
pub struct Kludge {
    peek_only: bool,
    last_balance: Option<(ContractId, AssetId, Word)>,
}

pub struct ShallowStorage {
    pub block_height: BlockHeight,
    pub timestamp: Tai64,
    pub consensus_parameters_version: u32,
    pub state_transition_version: u32,
    pub coinbase: fuel_vm::prelude::ContractId,
    pub storage_write_mask: RefCell<HashMap<&'static str, HashMap<Vec<u8>, Vec<u8>>>>,
    pub reads_by_column: RefCell<HashMap<String, Vec<StorageReadReplayEvent>>>,
    pub storage_reads: RefCell<Vec<StorageReadReplayEvent>>,
    pub kludge: RefCell<Kludge>,
    /// Client used to read data from the node as needed
    pub client: FuelClient,
}

impl ShallowStorage {
    fn initial_value_of_column(&self, column: &str, key: Vec<u8>) -> Option<Vec<u8>> {
        log::trace!("Attempting to read {} {}", column, hex::encode(&key));
        let reads = self.storage_reads.borrow();
        for r in reads.iter() {
            if r.column == column && &r.key == &key {
                log::trace!("-> ok {} {}", r.column, hex::encode(&r.key));
                return r.value.clone();
            }
        }
        panic!("No reads for {} {}", column, hex::encode(&key));
    }

    fn value_of_column(&self, column: &str, key: Vec<u8>) -> Option<Vec<u8>> {
        let writes = self.storage_write_mask.borrow();
        if let Some(value) = writes.get(column).and_then(|c| c.get(&key)) {
            return Some(value.clone());
        }
        self.initial_value_of_column(column, key)
    }

    fn replace_column(
        &self,
        column: &'static str,
        key: Vec<u8>,
        value: Vec<u8>,
    ) -> Option<Vec<u8>> {
        let mut writes = self.storage_write_mask.borrow_mut();
        let old_value = writes.entry(column).or_default().insert(key.clone(), value);
        if old_value.is_none() {
            self.initial_value_of_column(column, key)
        } else {
            old_value
        }
    }
}

macro_rules! storage_rw {
    ($table:ident, $convert_key:expr, $convert_value:expr, $convert_value_back:expr $(,)?) => {
        impl StorageSize<$table> for ShallowStorage {
            fn size_of_value(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<usize>, Self::Error> {
                log::debug!("{} size_of_value??? {}", stringify!($table), hex::encode(&$convert_key(key)));
                let head = self.value_of_column(stringify!($table), $convert_key(key));
                Ok(head.map(|v| v.len()))
            }
        }

        impl StorageInspect<$table> for ShallowStorage {
            type Error = Error;

            fn get(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<std::borrow::Cow<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>>, Self::Error> {
                log::debug!("{} get {}", stringify!($table), hex::encode(&$convert_key(key)));
                let head = self.value_of_column(stringify!($table), $convert_key(key));
                Ok(head.map($convert_value).map(std::borrow::Cow::Owned))
            }

            fn contains_key(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<bool, Self::Error> {
                log::debug!("{} contains_key {}", stringify!($table), hex::encode(&$convert_key(key)));
                let head = self.value_of_column(stringify!($table), $convert_key(key));
                Ok(head.is_some())
            }
        }

        impl StorageRead<$table> for ShallowStorage {
            fn read(
                &self,
                key: &<$table as fuel_vm::fuel_storage::Mappable>::Key,
                offset: usize,
                buf: &mut [u8],
            ) -> Result<Option<usize>, Self::Error> {
                log::debug!("{} read {}", stringify!($table), hex::encode(&$convert_key(key)),);
                let head = self.value_of_column(stringify!($table), $convert_key(key));
                let Some(value) = head else {
                    return Ok(None);
                };
                buf.copy_from_slice(&value[offset..][..buf.len()]);
                Ok(Some(buf.len()))
            }

            fn read_alloc(&self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<Vec<u8>>, Self::Error> {
                todo!("{} read_alloc {}", stringify!($table), hex::encode(&$convert_key(key)))
            }
        }

        impl StorageMutate<$table> for ShallowStorage {
            fn replace(
                &mut self,
                key: &<$table as fuel_vm::fuel_storage::Mappable>::Key,
                value: &<$table as fuel_vm::fuel_storage::Mappable>::Value,
            ) -> Result<Option<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>, Self::Error> {
                log::debug!("{} replace {} (value={value:?})", stringify!($table), hex::encode(&$convert_key(key)));
                Ok(self.replace_column(stringify!($table), $convert_key(key), $convert_value_back(value)).map($convert_value))
            }

            fn take(&mut self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<<$table as fuel_vm::fuel_storage::Mappable>::OwnedValue>, Self::Error> {
                todo!("{} take {}", stringify!($table), hex::encode(&$convert_key(key)))
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
                log::debug!("{} replace_bytes {key:?}", stringify!($table));
                let head = self.value_of_column(stringify!($table), $convert_key(key));
                Ok((buf.len(), head))
            }

            fn take_bytes(&mut self, key: &<$table as fuel_vm::fuel_storage::Mappable>::Key) -> Result<Option<Vec<u8>>, Self::Error> {
                todo!("take_bytes {key:?}")
            }
        }
    };
}

storage_rw!(
    ContractsRawCode,
    |key: &ContractId| -> Vec<u8> { (**key).to_vec() },
    |data| todo!("ContractsRawCode from bytes {data:?}"),
    |data| todo!("ContractsRawCode to bytes {data:?}"),
);
storage_rw!(
    ContractsState,
    |key: &ContractsStateKey| -> Vec<u8> { key.as_ref().into() },
    |data| { ContractsStateData(data) },
    |data: &[u8]| { data.to_vec() },
);
storage_rw!(
    ContractsAssets,
    |key: &ContractsAssetKey| -> Vec<u8> { key.as_ref().into() },
    |data| {
        assert_eq!(data.len(), 8);
        let mut buffer = [0u8; 8];
        buffer.copy_from_slice(&data);
        u64::from_be_bytes(buffer)
    },
    |data: &u64| data.to_be_bytes().to_vec(),
);
storage_rw!(
    UploadedBytecodes,
    |key: &Bytes32| -> Vec<u8> { key.as_ref().into() },
    |data| todo!("UploadedBytecodes from bytes {data:?}"),
    |data| todo!("UploadedBytecodes to bytes {data:?}"),
);
storage_rw!(
    BlobData,
    |key: &BlobId| -> Vec<u8> { key.as_ref().into() },
    |data| todo!("BlobData from bytes {data:?}"),
    |data| todo!("BlobData to bytes {data:?}"),
);

impl ContractsAssetsStorage for ShallowStorage {}

#[derive(Debug)]
pub enum Error {
    /// Failed to fetch data from the node
    NetworkError(io::Error),
    /// This block couldn't have been included
    InvalidBlock,
    Other(anyhow::Error),
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
impl From<anyhow::Error> for Error {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(e)
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
                todo!("timestamp {height:?}");
                // match self.client.block_by_height(height).await {
                //     Ok(Some(block)) => Ok(block.header.time.0),
                //     Ok(None) => Err(Error::InvalidBlock),
                //     Err(e) => Err(Error::NetworkError(e)),
                // }
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
                todo!("block_hash {block_height:?}");
                // match self.client.block_by_height(block_height).await {
                //     Ok(Some(block)) => Ok(block.id),
                //     Ok(None) => Err(Error::InvalidBlock),
                //     Err(e) => Err(Error::NetworkError(e)),
                // }
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
        log::debug!("contract_state_range {id:?} {start_key:?} {range:?}");
        let mut results = Vec::new();
        let mut key = U256::from_big_endian(start_key.as_ref());
        let mut key_buffer = Bytes32::zeroed();
        for offset in 0..(range as u64) {
            if offset != 0 {
                key = key
                    .checked_add(1.into())
                    .ok_or_else(|| anyhow!("range op exceeded available keyspace"))?;
            }

            key.to_big_endian(key_buffer.as_mut());
            let state_key = ContractsStateKey::new(id, &key_buffer.into());
            let value = self
                .storage::<fuel_vm::storage::ContractsState>()
                .get(&state_key)?;
            results.push(value);
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
        log::debug!("contract_state_insert_range {contract:?} {start_key:?}");
        // We need to return the number of keys that were previously unset
        // self.contract_state_range(contract, start_key, values.count())
        //     .map(|values| values.iter().filter(|v| dbg!(v).is_none()).count())

        let values: Vec<_> = values.collect();
        let mut key = U256::from_big_endian(start_key.as_ref());
        let mut key_buffer = Bytes32::zeroed();

        let mut found_unset = 0u32;
        for (idx, value) in values.iter().enumerate() {
            if idx != 0 {
                key = key
                    .checked_add(1.into())
                    .ok_or_else(|| anyhow!("range op exceeded available keyspace"))?;
            }

            key.to_big_endian(key_buffer.as_mut());
            let option = self.storage::<ContractsState>().replace(
                &(contract, Bytes32::from_bytes_ref(&key_buffer)).into(),
                value,
            )?;

            if option.is_none() {
                found_unset += 1;
            }
        }

        Ok(found_unset as usize)
    }

    fn contract_state_remove_range(
        &mut self,
        contract: &fuel_vm::prelude::ContractId,
        start_key: &fuel_vm::prelude::Bytes32,
        range: usize,
    ) -> Result<Option<()>, Self::DataError> {
        log::debug!("contract_state_remove_range {contract:?} {start_key:?}");
        if self
            .contract_state_range(contract, start_key, range)?
            .iter()
            .any(|v| v.is_none())
        {
            Ok(None)
        } else {
            Ok(Some(()))
        }
    }
}
