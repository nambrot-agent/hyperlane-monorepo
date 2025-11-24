# Rebalancer Enhancement: Pending Transfer Awareness

## Problem Statement

**Current Issue**: The rebalancer operates on static targets without awareness of pending incoming transfers. This causes insufficient rebalancing when large user transfers are in-flight.

**Example Scenario**:
- Arbitrum has 10k USDC collateral
- User initiates 70k USDC transfer Ethereum → Arbitrum (pending delivery)
- Rebalancer config target: 60k
- Rebalancer sees current balance (10k) and sends 50k to reach target
- **Result**: 70k transfer gets stuck (only 60k available, need 70k)

**Root Cause**: Rebalancer calculates deficit as `target - current` but should calculate as `max(target, pendingIncoming) - current`

## Goals

### Primary Goal
Make the rebalancer aware of pending incoming user transfers and adjust rebalancing amounts accordingly to prevent stuck transfers.

### Secondary Goal
Track rebalancing transfers separately so we can distinguish between user transfers (tracked via Explorer) and our own rebalances (tracked per method).

### Non-Goals (Future Work)
- Global cross-route optimization and netting
- Vault integration
- AI/ML strategies
- Inventory-based rebalancing via deposit/redeem

## High-Level Design

### Architecture

```
┌──────────────────────────────────────────────────────────┐
│                    RebalancerRunner                       │
│                     (existing)                            │
└───────────────────────────┬──────────────────────────────┘
                            │
                ┌───────────┼───────────┐
                │           │           │
                ▼           ▼           ▼
        ┌──────────┐  ┌──────────┐  ┌─────────────┐
        │ Monitor  │  │ Strategy │  │ NEW:        │
        │(existing)│  │(existing)│  │TransferState│
        └──────────┘  └──────────┘  └─────────────┘
                │           │           │
                └───────────┼───────────┘
                            ▼
                    ┌──────────────┐
                    │  Rebalancer  │
                    │  (modified)  │
                    └──────────────┘
```

### Components

#### 1. TransferState (NEW)
Centralized service that tracks:
- **User transfers**: Pending Hyperlane messages to each chain (from Explorer API)
- **Rebalance transfers**: Our own rebalancing operations (from execution tracking)

**Key Methods**:
```
getPendingUserTransfersTo(chain): bigint
  - Queries Explorer API for pending messages
  - Sums up amounts from message bodies

getPendingRebalancesTo(chain): bigint
  - Returns sum of our tracked rebalance transfers

getRequiredCollateral(chain): bigint
  - Returns: pendingUserTransfers - pendingRebalances
  - This is what's needed beyond current balance
```

#### 2. Modified Strategy
Strategies now receive both current balances AND pending transfer info.

**Change**: Strategy methods now take `TransferState` as parameter:
```
getRebalancingRoutes(rawBalances, transferState): Route[]
```

Strategies check if collateral is sufficient for pending demand:
```
requiredCollateral = transferState.getRequiredCollateral(chain)
if (currentCollateral < requiredCollateral) {
  deficit = requiredCollateral - currentCollateral
  // Add route to satisfy deficit
}
```

#### 3. Rebalancing Method Interface (NEW)
Different rebalancing methods (warp routes, CCTP, etc.) need different tracking approaches:

```
interface IRebalancingMethod {
  execute(route): receipt
  isPending(receipt): boolean  // Is this rebalance still in-flight?
}
```

**Implementations**:
- **WarpRouteMethod**: Tracks via Hyperlane message ID (Explorer API)
- **CCTPMethod**: Tracks indirectly by monitoring destination collateral increases
- **Future: InventoryMethod**: Tracks offchain bridge provider APIs

#### 4. RebalancingTracker (NEW)
Maintains map of pending rebalances and their status.

```
recordRebalance(receipt)
updateStatuses()  // Poll each method's isPending()
getPendingAmount(destination): bigint
```

### Data Flow

```
1. Monitor polls collateral balances every 60s
2. TransferState queries Explorer API for pending user transfers
3. Strategy calculates required rebalancing:
   - For each chain:
     - required = max(target, pendingUserTransfers - pendingRebalances)
     - deficit = required - current
     - If deficit > 0, create rebalancing route
4. Rebalancer executes routes
5. RebalancingTracker records each rebalance
6. Next iteration: TransferState includes tracked rebalances
```

## Implementation Details

### Explorer API Integration

**Endpoint** (need to confirm): `GET /messages?destination={domain}&status=pending`

**Parsing Message Amount**:
- Message body contains: `abi.encode(amount, recipient)`
- Decode to extract amount

### Tracking Rebalances

Each rebalancing method returns a receipt:
```typescript
{
  method: 'warp-route' | 'cctp',
  origin: string,
  destination: string,
  amount: bigint,
  timestamp: number,
  metadata: {
    // Method-specific tracking data
    // For warp: messageId
    // For cctp: txHash + collateralSnapshot
  }
}
```

**Warp Route Tracking**: Query Explorer API with message ID
**CCTP Tracking**: Compare destination collateral before/after, mark complete when increased

### Configuration Changes

Minimal - just add Explorer API URL:
```yaml
warpRouteId: USDC/canonical
explorerApiUrl: https://explorer.hyperlane.xyz/api  # NEW
strategy:
  # ... existing config unchanged
```

## Phased Implementation

### Phase 1: User Transfer Tracking (Week 1)
- [ ] Implement TransferState service
- [ ] Integrate Explorer API client
- [ ] Add to RebalancerRunner monitoring loop
- [ ] Unit tests with mock Explorer API

### Phase 2: Rebalance Tracking (Week 1-2)
- [ ] Define IRebalancingMethod interface
- [ ] Implement WarpRouteMethod with message tracking
- [ ] Implement CCTPMethod with collateral-based tracking
- [ ] Implement RebalancingTracker
- [ ] Unit tests for each method

### Phase 3: Strategy Integration (Week 2)
- [ ] Modify BaseStrategy to use TransferState
- [ ] Update WeightedStrategy
- [ ] Update MinAmountStrategy
- [ ] Integration tests

### Phase 4: Testing & Deployment (Week 3)
- [ ] End-to-end tests on testnet
- [ ] Monitor for stuck transfers
- [ ] Gradual mainnet rollout
- [ ] Metrics and dashboards

## Success Metrics

1. **Zero stuck transfers** due to insufficient collateral when rebalances are in-flight
2. **Accurate rebalancing amounts** that account for pending demand
3. **No over-rebalancing** (sending more than needed because we ignore pending rebalances)

## Open Questions

1. **Explorer API specifics**: What's the actual endpoint and response format?
2. **CCTP tracking reliability**: Is collateral-based tracking sufficient or do we need better method?
3. **Multiple pending to same destination**: How do we handle multiple concurrent rebalances to the same chain?
4. **Timeout handling**: When do we consider a pending rebalance "stuck" and retry?

## Future Extensions (Out of Scope)

- Cross-route netting via first-party route
- Inventory-based rebalancing (deposit/redeem)
- Vault integration (active vs earning liquidity)
- AI/ML-driven demand forecasting
- Global optimization across all routes
