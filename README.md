# Tracing + token flow analysis

### Primer

Direct ETH and token transfers are trivial to parse/filter just by looking at the tx input data + receipts but contract interactions can be complex to identify. Tracing allows us to dig deeper into the tx execution cycle to look through the internal calls and additional proxy contracts the tx interacts with.

To address the misclassifications in MEV-inspect, we're going to modify the existing inspectors to also conduct token flow analysis on the entire tx and estimate the MEV based on inflows/outflows. We can set a threshold for delta in the profits estimated by our inspectors and this method to flag the bigger misclassifications. This also allows us to make estimates without making protocol specific integrations. 

### Process

Aside from the EOA initiating the transaction itself, we check for balance changes for the `contract` (in the `to` field of the tx) that executes the arb/liquidation but also `proxy` contracts that can be identified by looking at the delegate calls (if any) in the tx traces. 

Types of traces (by `action_type`):

* `Call`, which happens when a method on the same contract or a different one is executed. We can identify the input parameters in each instance by looking at this trace. 
* `Self-destruct`, when a arbitrage contract destroys the code at its address and transfers the ETH held in the contract to an EOA. Common pattern among searchers given gas refunds. 
* `Create`, when a contract deploys another contract and transfers assets to it. 
* `Reward`, pertaining to the block reward and uncle reward, not relevant here. 

![](https://i.imgur.com/0VJiEbv.png)

More on delegate calls: https://medium.com/coinmonks/delegatecall-calling-another-contract-function-in-solidity-b579f804178c


The script iterates over all the traces and makes a note of all the ETH inflows/outflows as well as stablecoins (USDT/USDC/DAI) for the `eoa`, `contract`, `proxy`. Once it is done, it finds out net profit by subtracting the gas spent from the MEV revenue. Should be trivial to look for all ERC20 tokens that appear in the traces and dynamically calculate net flows for them as well, next thing on my priority after writing test cases for the current script. All profits will be converted to ETH, based on the exchange rate at that block height. 


### Examples 

**Simple Arb across DEXs**

Example: 
![](https://i.imgur.com/5ASjBDE.png)


https://etherscan.io/tx/0x4121ce805d33e952b2e6103a5024f70c118432fd0370128d6d7845f9b2987922

ETH=>ENG=>ETH across DEXs

Script output: 
EOA: 0x00000098163d8908dfbd126c873c9c4732a2c2e6
Contract: 0x000000000000006f6502b7f2bbac8c30a3f67e9a
Tx proxy: 0x0000000000000000000000000000000000000000
Stablecoins inflow/outflow: [0, 0]
Net ETH profit, Wei 22357881284770142 

^accurate and accounts for gas spent

**Arb flowing through stablecoin**

Example: 

![](https://i.imgur.com/kmNgRZ1.png)

https://etherscan.io/tx/0x496836e0bd1520388e36c79d587a31d4b3306e4f25352164178ca0667c7f9c29

Here the `contract` flows through the tether pool but only for the arb and they exit without any at the end. 

Script output: 

EOA: 0xcbb9feb9f882bb78b06ca335ee32f13a621c1a35
Contract: 0x000000000a2daefe11b26dcdaecde7d33ad03e9d
Tx proxy: 0x0000000000000000000000000000000000000000
Stablecoins inflow/outflow: [871, 871]
Net ETH profit, Wei 2848191823313317


**Arbs through multiple stablecoins**

Example: ![](https://i.imgur.com/zYuWld1.png)

https://etherscan.io/tx/0x5ab21bfba50ad3993528c2828c63e311aafe93b40ee934790e545e150cb6ca73

Script output: 

EOA: 0x46eaadc8f2199463db26d1797131900575f0d264
Contract: 0x5a48ae20173382884929dd5e130ed9b81931ea88
Tx proxy: 0x0000000000000000000000000000000000000000
Stablecoins inflow/outflow: [0, 0]
Net ETH profit, Wei 46223431362651237

Here we notice the stablecoin inflow/outflow to be 0 despite flowing through them, this is because for those trades, the coins are transferred between the router and not the eao/contract/proxy, so they do not have to be accounted. We still get the net MEV profit accurately without having to look at the different tokens involved. 
