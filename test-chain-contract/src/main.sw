contract;


abi MyContract {
    fn invoke(times: u64);
}


abi Counter {
    #[storage(read, write)]
    fn increment() -> u64;

    #[storage(read)]
    fn count() -> u64;

    #[storage(write)]
    fn clear();
}


impl MyContract for Contract {
    fn invoke(times: u64) {
        let contract_id = 0xfc51eadbb8962d9ac7a22e887f82abcf797457bb99fac8a2f634cf1fa08284c8;
        let mut times = times;
        while times > 0 {
            let counter = abi(Counter, contract_id);
            let _ = counter.increment();
            times -= 1;
        }
    }
}
