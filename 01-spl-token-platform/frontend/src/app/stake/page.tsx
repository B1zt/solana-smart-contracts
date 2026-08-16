'use client';

import {useWallet} from '@solana/wallet-adapter-react';
import {useQuery} from '@tanstack/react-query';
import {useState} from 'react';
import {api, formatAmount, formatCountdown, shortAddress} from '@/lib/api';
import {cn} from '@/lib/cn';

export default function StakePage() {
  const {publicKey} = useWallet();
  const [selected, setSelected] = useState<string | null>(null);

  const {data: pools, isLoading} = useQuery({
    queryKey: ['pools'],
    queryFn: api.pools,
    refetchInterval: 30_000,
  });

  const activePool = selected ?? pools?.pools[0]?.address ?? null;

  const {data: position} = useQuery({
    queryKey: ['position', activePool, publicKey?.toBase58()],
    queryFn: () => api.position(activePool!, publicKey!.toBase58()),
    enabled: Boolean(activePool && publicKey),
    refetchInterval: 15_000,
  });

  const pool = pools?.pools.find((entry) => entry.address === activePool);

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Stake</h1>
        <p className="max-w-2xl text-neutral-400">
          Rewards accrue through a single running accumulator rather than per-user bookkeeping, so
          the cost of a distribution does not grow with the number of stakers. On Solana that is
          structural: a transaction touches a bounded set of accounts, so anything that iterates
          over participants stops working once there are enough of them.
        </p>
      </header>

      {isLoading ? (
        <div className="h-40 animate-pulse rounded-xl bg-neutral-900" />
      ) : (pools?.pools.length ?? 0) === 0 ? (
        <p className="py-24 text-center text-neutral-600">No staking pools deployed yet.</p>
      ) : (
        <>
          {(pools!.pools.length > 1) && (
            <div className="flex flex-wrap gap-2">
              {pools!.pools.map((entry) => (
                <button
                  key={entry.address}
                  type="button"
                  onClick={() => setSelected(entry.address)}
                  className={cn(
                    'rounded-lg border px-3 py-2 text-sm transition-colors',
                    entry.address === activePool
                      ? 'border-violet-500 bg-violet-500/10 text-violet-300'
                      : 'border-neutral-800 text-neutral-400 hover:border-neutral-700',
                  )}
                >
                  {shortAddress(entry.stakeMint)}
                </button>
              ))}
            </div>
          )}

          {pool && (
            <>
              <div className="grid gap-4 sm:grid-cols-3">
                <div className="card">
                  <p className="text-xs uppercase tracking-wide text-neutral-500">Total staked</p>
                  <p className="mt-1 text-2xl font-semibold tabular-nums">
                    {formatAmount(pool.totalStaked)}
                  </p>
                </div>
                <div className="card">
                  <p className="text-xs uppercase tracking-wide text-neutral-500">APR</p>
                  <p className="mt-1 text-2xl font-semibold tabular-nums">
                    {(pool.aprBps / 100).toFixed(2)}%
                  </p>
                </div>
                <div className="card">
                  <p className="text-xs uppercase tracking-wide text-neutral-500">Rewards end</p>
                  <p className="mt-1 text-2xl font-semibold tabular-nums">
                    {formatCountdown(pool.rewardEndTs) ?? 'ended'}
                  </p>
                </div>
              </div>

              <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_380px]">
                <div className="card space-y-4">
                  <h2 className="font-medium">Your position</h2>

                  {!publicKey ? (
                    <p className="py-6 text-center text-sm text-neutral-500">Connect a wallet.</p>
                  ) : !position ? (
                    <p className="py-6 text-center text-sm text-neutral-600">
                      Nothing staked in this pool yet.
                    </p>
                  ) : (
                    <>
                      <dl className="grid grid-cols-2 gap-4 text-sm">
                        <div>
                          <dt className="text-neutral-500">Staked</dt>
                          <dd className="text-xl font-semibold tabular-nums">
                            {formatAmount(position.amount)}
                          </dd>
                        </div>
                        <div>
                          <dt className="text-neutral-500">Pending rewards</dt>
                          <dd className="text-xl font-semibold tabular-nums text-emerald-400">
                            {formatAmount(position.pendingReward)}
                          </dd>
                        </div>
                      </dl>

                      {BigInt(position.unstakingAmount) > 0n && (
                        <div className="rounded-lg border border-amber-900 bg-amber-950/30 px-3 py-2 text-sm text-amber-300">
                          {formatAmount(position.unstakingAmount)} unstaking
                          {position.unstakeReadyTs && (
                            <>
                              , withdrawable in{' '}
                              {formatCountdown(position.unstakeReadyTs) ?? 'now'}
                            </>
                          )}
                        </div>
                      )}
                    </>
                  )}
                </div>

                <div className="card space-y-4">
                  <h2 className="font-medium">Pool details</h2>
                  <dl className="space-y-2 text-sm">
                    <div className="flex justify-between">
                      <dt className="text-neutral-500">Stake mint</dt>
                      <dd className="font-mono text-xs">{shortAddress(pool.stakeMint)}</dd>
                    </div>
                    <div className="flex justify-between">
                      <dt className="text-neutral-500">Reward mint</dt>
                      <dd className="font-mono text-xs">{shortAddress(pool.rewardMint)}</dd>
                    </div>
                    <div className="flex justify-between">
                      <dt className="text-neutral-500">Cooldown</dt>
                      <dd>
                        {pool.cooldownSeconds === 0
                          ? 'None'
                          : `${Math.round(pool.cooldownSeconds / 3_600)}h`}
                      </dd>
                    </div>
                  </dl>

                  <p className="border-t border-neutral-800 pt-3 text-xs text-neutral-600">
                    Unstaking moves tokens out of the earning balance and starts the cooldown.
                    Rewards already accrued stay claimable throughout.
                  </p>
                </div>
              </div>
            </>
          )}
        </>
      )}
    </div>
  );
}
