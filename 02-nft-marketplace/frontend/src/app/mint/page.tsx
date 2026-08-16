'use client';

import {useWallet} from '@solana/wallet-adapter-react';
import {useQuery} from '@tanstack/react-query';
import {useState} from 'react';
import {api, formatSol} from '@/lib/api';
import {cn} from '@/lib/cn';
import {formatCountdown} from '@/lib/format';

export default function MintPage() {
  const {publicKey} = useWallet();
  const [selected, setSelected] = useState<string | null>(null);

  const {data} = useQuery({queryKey: ['collections'], queryFn: api.collections});

  const collections = data?.collections ?? [];
  const collection = collections.find((entry) => entry.address === selected) ?? collections[0];

  const now = Date.now();

  // The live phase, or the next one due to open.
  const activePhase =
    collection?.phases.find(
      (phase) =>
        now >= new Date(phase.startTs).getTime() && now < new Date(phase.endTs).getTime(),
    ) ??
    collection?.phases.find((phase) => now < new Date(phase.startTs).getTime()) ??
    collection?.phases[0];

  const {data: proof} = useQuery({
    queryKey: ['allowlist', activePhase?.address, publicKey?.toBase58()],
    queryFn: () => api.allowlistProof(activePhase!.address, publicKey!.toBase58()),
    // A public phase has no allowlist to check.
    enabled: Boolean(
      activePhase && publicKey && activePhase.merkleRoot !== '0'.repeat(64),
    ),
  });

  const isPublic = activePhase?.merkleRoot === '0'.repeat(64);
  const live =
    activePhase &&
    now >= new Date(activePhase.startTs).getTime() &&
    now < new Date(activePhase.endTs).getTime();

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Mint</h1>
        <p className="max-w-2xl text-neutral-400">
          Each phase is its own account rather than a field on the collection, so unrelated mints do
          not contend on writing the same state. That is what Solana&apos;s account model is for.
        </p>
      </header>

      {collections.length === 0 ? (
        <p className="py-24 text-center text-neutral-600">No collections deployed yet.</p>
      ) : (
        <>
          {collections.length > 1 && (
            <div className="flex flex-wrap gap-2">
              {collections.map((entry) => (
                <button
                  key={entry.address}
                  type="button"
                  onClick={() => setSelected(entry.address)}
                  className={cn(
                    'rounded-lg border px-3 py-2 text-sm transition-colors',
                    entry.address === collection?.address
                      ? 'border-violet-500 bg-violet-500/10 text-violet-300'
                      : 'border-neutral-800 text-neutral-400 hover:border-neutral-700',
                  )}
                >
                  {entry.name}
                </button>
              ))}
            </div>
          )}

          {collection && (
            <div className="grid gap-6 lg:grid-cols-[minmax(0,1fr)_380px]">
              <div className="card space-y-4">
                <h2 className="font-medium">{collection.name}</h2>

                <div>
                  <div className="mb-2 flex items-baseline justify-between text-sm">
                    <span className="text-neutral-500">Minted</span>
                    <span className="tabular-nums">
                      {collection.minted.toLocaleString()} /{' '}
                      {collection.maxSupply.toLocaleString()}
                    </span>
                  </div>
                  <div className="h-2 overflow-hidden rounded-full bg-neutral-800">
                    <div
                      className="h-full rounded-full bg-violet-500 transition-[width] duration-500"
                      style={{
                        width: `${Math.min(100, (collection.minted / collection.maxSupply) * 100)}%`,
                      }}
                    />
                  </div>
                </div>

                <div className="space-y-2 border-t border-neutral-800 pt-4">
                  <h3 className="text-sm font-medium">Phases</h3>
                  {collection.phases.map((phase) => {
                    const phaseLive =
                      now >= new Date(phase.startTs).getTime() &&
                      now < new Date(phase.endTs).getTime();

                    return (
                      <div
                        key={phase.address}
                        className="flex items-center justify-between rounded-lg border border-neutral-800 px-3 py-2 text-sm"
                      >
                        <div>
                          <p>
                            {phase.merkleRoot === '0'.repeat(64) ? 'Public' : 'Allowlist'} phase{' '}
                            {phase.index + 1}
                          </p>
                          <p className="text-xs text-neutral-500">
                            {formatSol(phase.price)} · {phase.minted} minted
                          </p>
                        </div>
                        <span
                          className={cn(
                            'rounded-full px-2 py-0.5 text-xs',
                            phaseLive
                              ? 'bg-emerald-500/15 text-emerald-300'
                              : 'bg-neutral-800 text-neutral-500',
                          )}
                        >
                          {phaseLive
                            ? (formatCountdown(phase.endTs) ?? 'ending')
                            : now < new Date(phase.startTs).getTime()
                              ? `in ${formatCountdown(phase.startTs) ?? 'moments'}`
                              : 'ended'}
                        </span>
                      </div>
                    );
                  })}
                </div>
              </div>

              <div className="card space-y-4">
                {!activePhase ? (
                  <p className="text-sm text-neutral-500">No phases configured.</p>
                ) : (
                  <>
                    <div className="flex items-baseline justify-between">
                      <span className="text-sm text-neutral-500">Price</span>
                      <span className="text-2xl font-semibold tabular-nums">
                        {activePhase.price === '0' ? 'Free' : formatSol(activePhase.price)}
                      </span>
                    </div>

                    {!isPublic && publicKey && (
                      <div
                        className={cn(
                          'rounded-lg border px-3 py-2 text-sm',
                          proof
                            ? 'border-emerald-900 bg-emerald-950/30 text-emerald-300'
                            : 'border-amber-900 bg-amber-950/30 text-amber-300',
                        )}
                      >
                        {proof
                          ? `You are on the allowlist, allowance ${proof.allowance}.`
                          : 'This wallet is not on the allowlist for this phase.'}
                      </div>
                    )}

                    <button
                      type="button"
                      className="btn-primary w-full"
                      disabled={!publicKey || !live || (!isPublic && !proof)}
                    >
                      {!publicKey
                        ? 'Connect wallet'
                        : !live
                          ? 'Phase not active'
                          : !isPublic && !proof
                            ? 'Not eligible'
                            : 'Mint'}
                    </button>

                    <p className="text-xs text-neutral-600">
                      Minting creates the NFT mint, an associated token account and a per-wallet
                      receipt account. On Solana every piece of state needs an account, and the
                      minter pays the rent for theirs.
                    </p>
                  </>
                )}
              </div>
            </div>
          )}
        </>
      )}
    </div>
  );
}
