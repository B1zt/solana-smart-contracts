'use client';

import {useConnection, useWallet} from '@solana/wallet-adapter-react';
import {useQuery} from '@tanstack/react-query';
import {useState} from 'react';
import {api, formatAmount, formatCountdown, shortAddress} from '@/lib/api';
import {cn} from '@/lib/cn';

/**
 * Airdrop claim page.
 *
 * The distributor address is an input rather than a constant, because one deployment of this
 * program can host many airdrops: the distributor PDA is seeded by mint and authority, so a project
 * running two campaigns has two of them.
 */
export default function ClaimPage() {
  const {publicKey} = useWallet();
  const {connection} = useConnection();

  const [distributorInput, setDistributorInput] = useState('');
  const [distributor, setDistributor] = useState('');

  const {data: claim, isLoading} = useQuery({
    queryKey: ['claim', distributor, publicKey?.toBase58()],
    queryFn: () => api.claim(distributor, publicKey!.toBase58()),
    enabled: Boolean(distributor && publicKey),
  });

  const {data: stats} = useQuery({
    queryKey: ['airdropStats', distributor],
    queryFn: () => api.airdropStats(distributor),
    enabled: Boolean(distributor),
    retry: false,
  });

  const {data: fee} = useQuery({
    queryKey: ['priorityFee'],
    queryFn: () => api.priorityFee(),
    refetchInterval: 30_000,
  });

  const claimedPercent =
    stats && stats.entryCount > 0 ? (stats.claimedCount / stats.entryCount) * 100 : 0;

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Airdrop</h1>
        <p className="max-w-2xl text-neutral-400">
          Claims are verified against a Merkle root stored on-chain. Each claim creates a small
          account whose very existence is the claimed flag, so the runtime rejects a second claim
          before the program checks anything.
        </p>
      </header>

      <form
        className="flex flex-wrap gap-3"
        onSubmit={(event) => {
          event.preventDefault();
          setDistributor(distributorInput.trim());
        }}
      >
        <input
          className="input flex-1 font-mono text-sm"
          placeholder="Distributor address"
          value={distributorInput}
          onChange={(event) => setDistributorInput(event.target.value)}
        />
        <button type="submit" className="btn-primary">
          Look up
        </button>
      </form>

      {stats && (
        <div className="card space-y-3">
          <div className="flex items-baseline justify-between">
            <span className="text-sm text-neutral-500">Claimed</span>
            <span className="tabular-nums">
              {stats.claimedCount.toLocaleString()} / {stats.entryCount.toLocaleString()} wallets
            </span>
          </div>

          <div className="h-2 overflow-hidden rounded-full bg-neutral-800">
            <div
              className="h-full rounded-full bg-violet-500 transition-[width] duration-500"
              style={{width: `${claimedPercent}%`}}
            />
          </div>

          <div className="flex justify-between text-sm text-neutral-500">
            <span>{formatAmount(stats.totalAmount)} allocated</span>
            {stats.deadline && (
              <span>Closes in {formatCountdown(stats.deadline) ?? 'passed'}</span>
            )}
          </div>
        </div>
      )}

      <div className="card space-y-4">
        {!publicKey ? (
          <p className="py-8 text-center text-neutral-500">Connect a wallet to check eligibility.</p>
        ) : !distributor ? (
          <p className="py-8 text-center text-neutral-500">
            Enter a distributor address to check your allocation.
          </p>
        ) : isLoading ? (
          <p className="py-8 text-center text-neutral-500">Checking eligibility…</p>
        ) : !claim ? (
          <div className="py-8 text-center">
            <p className="text-neutral-300">This wallet has no allocation.</p>
            <p className="mt-1 text-sm text-neutral-500">
              Allocations were snapshotted before the airdrop opened.
            </p>
          </div>
        ) : (
          <>
            <div className="flex items-baseline justify-between">
              <span className="text-sm text-neutral-500">Your allocation</span>
              <span className="text-3xl font-semibold tabular-nums">
                {formatAmount(claim.amount)}
              </span>
            </div>

            <dl className="grid grid-cols-2 gap-4 border-t border-neutral-800 pt-4 text-sm">
              <div>
                <dt className="text-neutral-500">Leaf index</dt>
                <dd className="tabular-nums">#{claim.index}</dd>
              </div>
              <div>
                <dt className="text-neutral-500">Proof length</dt>
                <dd className="tabular-nums">{claim.proof.length} nodes</dd>
              </div>
              <div className="col-span-2">
                <dt className="text-neutral-500">Claim status account</dt>
                <dd className="font-mono text-xs">{shortAddress(claim.claimStatusPda)}</dd>
              </div>
            </dl>

            {claim.claimed ? (
              <p className="rounded-lg border border-emerald-900 bg-emerald-950/30 px-4 py-3 text-sm text-emerald-300">
                Already claimed. The claim status account exists on-chain, so the runtime will reject
                any further attempt.
              </p>
            ) : (
              <>
                <button type="button" className="btn-primary w-full" disabled>
                  Claim {formatAmount(claim.amount)}
                </button>
                <p className="text-xs text-neutral-500">
                  Building the claim transaction requires the program IDL and the claimant&apos;s
                  associated token account. The proof and index above are everything the instruction
                  needs; see <code>backend/src/chain/transactions.ts</code> for the submission path
                  with priority fees and blockhash refresh.
                </p>
              </>
            )}
          </>
        )}
      </div>

      {fee && (
        <div className="card">
          <h2 className="mb-2 text-sm font-medium">Network conditions</h2>
          <p className="text-sm text-neutral-400">
            Recent transactions touching this program paid{' '}
            <span className="tabular-nums text-neutral-200">
              {fee.microLamportsPerComputeUnit.toLocaleString()}
            </span>{' '}
            micro-lamports per compute unit, roughly{' '}
            <span className="tabular-nums text-neutral-200">
              {(fee.estimatedCostLamports / 1e9).toFixed(6)} SOL
            </span>{' '}
            for a typical transaction.
          </p>
          <p className="mt-2 text-xs text-neutral-600">
            Solana&apos;s base fee is fixed and does no ordering work under contention, so attaching
            a priority fee is what determines whether a transaction lands when the network is busy.
          </p>
        </div>
      )}

      <p
        className={cn(
          'text-center text-xs',
          connection.rpcEndpoint.includes('devnet') ? 'text-neutral-600' : 'text-amber-500',
        )}
      >
        Connected to {connection.rpcEndpoint}
      </p>
    </div>
  );
}
