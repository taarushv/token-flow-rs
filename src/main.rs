use ethers::providers::{Http, Middleware, Provider};
use ethers::types::{TxHash, Transaction, Trace, TransactionReceipt, CallType, Action, Address, U256};
use std::{convert::TryFrom};

const ARCHIVE_RPC:&str = "https://dashboard.flashbots.net/eth-sJrVNk4Xoa";
// Interacts with tether, usdc, weth
// arb b/w 0x and curve
const TEST_TX:&str = "0x5ab21bfba50ad3993528c2828c63e311aafe93b40ee934790e545e150cb6ca73"; // Test tx to verify token flows
const WEI_IN_ETHER: U256 = U256([0x0de0b6b3a7640000, 0x0, 0x0, 0x0]);
// Relevant contracts
const WETH:&str = "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2";
const USDC:&str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
const USDT:&str = "0xdAC17F958D2ee523a2206206994597C13D831ec7";
const DAI:&str = "0x6B175474E89094C44Da98b954EedeAC495271d0F";

//const ETH_PRICE:f64 = 1471.5439;

//const EXCLUDED_CONTRACTS: [&str; 1] = ["0x3d71d79c224998e608d03c5ec9b405e7a38505f0"]; // KeeperDAO which whitelists who can extract MEV

#[tokio::main]
async fn main() {
    let provider = Provider::<Http>::try_from(
        ARCHIVE_RPC,
    )
    .unwrap();
    core(provider).await
}

async fn get_tx_data<M: Middleware + Clone + 'static>(provider: M, tx: TxHash) -> Transaction{
    return provider.get_transaction(tx).await.unwrap().unwrap();
}

async fn get_tx_traces<M: Middleware + Clone + 'static>(provider: M, tx:TxHash) -> Vec<Trace>{
    return provider.trace_transaction(tx).await.unwrap();
}

async fn get_tx_receipt<M: Middleware + Clone + 'static>(provider: M, tx:TxHash) -> TransactionReceipt {
    return provider.get_transaction_receipt(tx).await.unwrap().unwrap();
}


// A bot can have a contract (that it initially calls) AND a proxy contract (that the initial contract triggers via DelegateCall)
// that engage in extracting MEV, We find the proxy implementation (if any) to see tokenflows on them
fn get_proxy_impl(tx_traces:Vec<Trace>, contract:Address) -> Address {
    let mut proxy_impl:Address = Address::zero();
    for trace in tx_traces.iter(){
        match &trace.action {
            Action::Call(call) => {
                if proxy_impl == Address::zero() && call.call_type == CallType::DelegateCall && call.from == contract{
                    proxy_impl= call.to; // TODO: what if they use multiple proxies?
                }
            }
            _ => continue, // we skip over other action types as we only care about the proxy (if any)
        }
    }
    return proxy_impl;
}

fn crop_address(s: &mut String, pos: usize) {
    match s.char_indices().nth(pos) {
        Some((pos, _)) => {
            s.drain(..pos);
        }
        None => {
            s.clear();
        }
    }
}

// ETH_GET - done
// ETH_GIVE - done
// WETH_GET1 - done
// WETH_GET2 - done
// WETH_GIVE1 - done
// WETH_GIVE2 - done
// ETH_SELFDESTRUCT - Done

fn get_ether_flows(tx_traces:Vec<Trace>, eoa: Address, contract: Address, proxy: Address) -> [U256; 2] {
    let mut eth_inflow= U256::zero();
    let mut eth_outflow = U256::zero();
    for trace in tx_traces.iter(){
        match &trace.action {
            Action::Call(call) => {
                // ETH_GET
                // Check if the call transfers value, isn't from WETH (to avoid double counting) and transfers to one of the relevant addresses
                if call.value > U256::zero() && call.call_type != CallType::DelegateCall && call.from != WETH.parse::<Address>().unwrap() && 
                (call.to == eoa || call.to == contract || call.to == proxy)
                {
                    eth_inflow += call.value;
                }

                // ETH_GIVE
                // Check if the call transfers value, isn't from WETH and transfers ETH out of one of the relevant addresses
                if call.value > U256::zero() && call.call_type != CallType::DelegateCall && call.to != WETH.parse::<Address>().unwrap() && 
                (call.from == eoa || call.from == contract || call.from == proxy){
                    eth_outflow += call.value;
                }

                // WETH_GET1 & WETH_GET2
                // WETH_GIVE1 & WETH_GIVE2
                if call.to == WETH.parse::<Address>().unwrap() {
                    let data = call.input.as_ref();
                    if data.len()==68 { // 4 bytes of function identifier + 64 bytes of params
                        let fn_signature = hex::encode(&data[..4]);
                        if fn_signature == "a9059cbb" { // transfer(address to,uint256 value )
                            let value = U256::from(&data[36..68]); // WETH amount
                            let mut address = hex::encode(&data[4..36]);
                            crop_address(&mut address, 24);
                            let prefix: &str = "0x";
                            let final_address = format!("{}{}", prefix, address).parse::<Address>().unwrap();
                            if final_address == eoa || final_address == contract || final_address == proxy { // Might have to exclude direct calls to uniswap router
                                // Once we confirm that the traces contain a WETH transfer to one of the relevant address
                                // we count that towards the inflow
                                eth_inflow  += value;
                            } else if call.from == eoa || call.from == contract || call.from == proxy {
                                // If the WETH flows from the searchers accounts/contracts. 
                                eth_outflow += value;
                            }
                        }
                    }
                    if data.len() == 100 {
                        let fn_signature = hex::encode(&data[..4]);
                        if fn_signature == "23b872dd" { // transferFrom(address from,address to,uint256 value )
                            let value = U256::from(&data[68..100]); // WETH amount
                            let mut to_address = hex::encode(&data[36..68]);
                            let mut from_address = hex::encode(&data[4..36]);
                            crop_address(&mut to_address, 24);
                            crop_address(&mut from_address, 24);
                            let prefix: &str = "0x";
                            let final_to_address = format!("{}{}", prefix, to_address).parse::<Address>().unwrap();
                            let final_from_address = format!("{}{}", prefix, to_address).parse::<Address>().unwrap();
                            if final_to_address == eoa || final_to_address == contract || final_to_address == proxy { // Might have to exclude direct calls to uniswap router
                                // Once we confirm that the traces contain a WETH transfer to one of the relevant address
                                // we count that towards the inflow
                                eth_inflow  += value;
                            } else if final_from_address == eoa || final_from_address == contract || final_from_address == proxy {
                                // Vice versa
                                eth_outflow += value;
                            }
                        }
                    }
                }
            }
            // ETH_SELFDESTRUCT
            Action::Suicide(suicide) => { // The OP code was renamed to "Self-destruct" but OpenEthereum still uses the old ref 
                // If a trace calls self destruct, transferring the funds to either the eoa/contract/proxy
                // we count the ETH transferred out towards our net inflows
                if suicide.refund_address == eoa || suicide.refund_address == contract || suicide.refund_address == proxy {
                    eth_inflow += suicide.balance;
                }

                // What if they transfer the funds out to an arbitrary address that's not any of the addresses?
                // i.e If a searcher uses a cold storage address to transfer out the arb profits
            }
            _ => {
                // we ignore the case for action type Create/Reward as it doesn't pertain to eth inflows or outflows
                continue
            }
        }
    }
    if eth_outflow > U256::zero() && eth_inflow >U256::zero() {
        return [eth_inflow , eth_outflow];
    }
    return [U256::zero(), U256::zero()];
}

fn get_stablecoin_flows(tx_traces: Vec<Trace>, eoa:Address, contract: Address, proxy: Address) ->  [U256; 2] {
    let mut dollar_inflow= U256::zero();
    let mut dollar_outflow = U256::zero();
    for trace in tx_traces.iter(){
        match &trace.action {
            Action::Call(call) => {
                // USD_GET1 & USD_GET2
                // USD_GIVE1 & USD_GIVE2
                if call.to == USDC.parse::<Address>().unwrap() || call.to == USDT.parse::<Address>().unwrap() || call.to == DAI.parse::<Address>().unwrap(){
                    let data = call.input.as_ref();
                    if data.len()==68 { // 4 bytes of function identifier + 64 bytes of params
                        let fn_signature = hex::encode(&data[..4]);
                        if fn_signature == "a9059cbb" { // transfer(address to,uint256 value )
                            let value = U256::from(&data[36..68]); // USD amount
                            let mut address = hex::encode(&data[4..36]);
                            crop_address(&mut address, 24);
                            let prefix: &str = "0x";
                            let final_address = format!("{}{}", prefix, address).parse::<Address>().unwrap();
                            if final_address == eoa || final_address == contract || final_address == proxy { // Might have to exclude direct calls to uniswap router
                                // Once we confirm that the traces contain a USD transfer to one of the relevant address
                                // we count that towards the inflow
                                if call.to != DAI.parse::<Address>().unwrap(){ // DAI has 18 digits while USDT/USDC have 6
                                    dollar_inflow += value/U256::from(1000000);
                                }else {
                                    //dollar_inflow += value/U256::from();
                                    dollar_inflow += value/WEI_IN_ETHER;
                                }
                            } else if call.from == eoa || call.from == contract || call.from == proxy {
                                // If the USD flows from the searchers accounts/contracts. 
                                if call.to != DAI.parse::<Address>().unwrap(){ // DAI has 18 digits while USDT/USDC have 6
                                    dollar_outflow += value/U256::from(1000000);
                                }else {
                                    //dollar_inflow += value/U256::from();
                                    dollar_outflow += value/WEI_IN_ETHER;
                                }
                            }
                        }
                    }
                    if data.len() == 100 {
                        let fn_signature = hex::encode(&data[..4]);
                        if fn_signature == "23b872dd" { // transferFrom(address from,address to,uint256 value )
                            let value = U256::from(&data[68..100]); // USD amount
                            let mut to_address = hex::encode(&data[36..68]);
                            let mut from_address = hex::encode(&data[4..36]);
                            crop_address(&mut to_address, 24);
                            crop_address(&mut from_address, 24);
                            let prefix: &str = "0x";
                            let final_to_address = format!("{}{}", prefix, to_address).parse::<Address>().unwrap();
                            let final_from_address = format!("{}{}", prefix, to_address).parse::<Address>().unwrap();
                            if final_to_address == eoa || final_to_address == contract || final_to_address == proxy { // Might have to exclude direct calls to uniswap router
                                // Once we confirm that the traces contain a USD transfer to one of the relevant address
                                // we count that towards the inflow
                                if call.to != DAI.parse::<Address>().unwrap(){ // DAI has 18 digits while USDT/USDC have 6
                                    dollar_inflow += value/U256::from(1000000);
                                }else {
                                    dollar_inflow += value/WEI_IN_ETHER;
                                }
                            } else if final_from_address == eoa || final_from_address == contract || final_from_address == proxy {
                                // Vice versa
                                if call.to != DAI.parse::<Address>().unwrap(){ // DAI has 18 digits while USDT/USDC have 6
                                    dollar_outflow += value/U256::from(1000000);
                                }else {
                                    dollar_outflow += value/WEI_IN_ETHER;
                                }
                            }
                        }
                    }
                }
            }
            _ => {
                // we ignore the case for action type Create/Reward/Self-destruct as it doesn't pertain to eth inflows or outflows
                continue
            }
        }
    }
    if dollar_outflow > U256::zero() && dollar_inflow >U256::zero() {
        return [dollar_inflow , dollar_outflow];
    }
    return [U256::zero(), U256::zero()];
}




async fn parse_token_flow_from_traces(tx_traces:Vec<Trace>,eoa:Address, contract:Address, proxy:Address) {
    println!("{} {} {}", eoa, contract, proxy);
    println!("{}", tx_traces.len())
}



async fn core<M: Middleware + Clone + 'static>(provider: M) {
    
    // Get latest block number to make sure we're hooked up to the node
    let tx_hash = TEST_TX.parse::<TxHash>().unwrap();
    

    let tx_data = get_tx_data(provider.clone(), tx_hash).await;

    let tx_receipt = get_tx_receipt(provider.clone(), tx_hash).await;
    //let gas_used_in_wei = tx_receipt.gas_used.unwrap();

    //let cost_in_wei = gas_used_in_wei * tx_data.gas_price; // "cost"
    let eoa = tx_data.from; // searcher address
    let contract = tx_data.to.unwrap(); // 
    //let eth_price = ETH_PRICE;
    //let mut revenue_parsed:f64 = 0.0;

    //let gas_used = tx_recepit.gas_used.unwrap();
    //let gas_cost = format_units(tx_data.gas_price * tx_data.gas, 9);
    //let gas_cost = (tx_data.gas * tx_data.gas_price)/base.pow(18);
    //println!("Tx data:  {:?}", tx_data);
    
    let tx_traces = get_tx_traces(provider.clone(), tx_hash).await;
    let proxy = get_proxy_impl(tx_traces.clone(), contract);
    println!("Tx proxy: {:?}", proxy);
    //parse_token_flow_from_traces().await;
    //parse_token_flow_from_traces(tx_traces.clone(), eoa, contract, proxy).await;
    println!("Eth inflow/outflow: {:?}",get_ether_flows(tx_traces.clone(), eoa, contract, proxy));
    println!("Stablecoins inflow/outflow: {:?}", get_stablecoin_flows(tx_traces.clone(), eoa, contract, proxy))
}


