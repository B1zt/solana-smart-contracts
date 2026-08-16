'use client';

import {useQuery} from '@tanstack/react-query';
import Link from 'next/link';
import {api, shortAddress} from '@/lib/api';

export default function HomePage() {
  const {data: status} = useQuery({
    queryKey: ['chainStatus'],
    queryFn: api.chainStatus,
    refetchInterval: 10_000,
    retry: false,
  });

  const {data: pools} = useQuery({queryKey: ['pools'], queryFn: api.pools, retry: false});

  return (
    <div className="space-y-12">
      <section className="space-y-3">
        <h1 className="text-4xl font-semibold tracking-tight">Solana token platform</h1>
        <p className="max-w-2xl text-neutral-400">
          A capped SPL mint whose authority is a program-derived address, linear vesting with
          cliffs, a Merkle airdrop, and staking. One Anchor program, built around the primitives
          Solana actually gives you rather than a translation of Ethereum patterns.
        </p>
      </section>

      <section className="grid gap-4 sm:grid-cols-3">
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">Cluster</p>
          <p className="mt-1 text-2xl font-semibold capitalize">{status?.cluster ?? '-'}</p>
        </div>
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">Slot</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">
            {status?.slot.toLocaleString() ?? '-'}
          </p>
        </div>
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">Staking pools</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{pools?.pools.length ?? '-'}</p>
        </div>
      </section>

      <section className="grid gap-4 sm:grid-cols-3">
        {[
          {
            href: '/claim',
            title: 'Claim an airdrop',
            body: 'Merkle-verified claims where each claim creates an account. Its existence is the claimed flag, so the runtime rejects a replay before the program checks anything.',
          },
          {
            href: '/vesting',
            title: 'Track vesting',
            body: 'One program-owned vault per grant, funded at creation. Releasing is permissionless but the destination is constrained to the beneficiary on-chain.',
          },
          {
            href: '/stake',
            title: 'Stake for rewards',
            body: 'A single reward accumulator instead of per-user bookkeeping, because a Solana transaction cannot iterate over an unbounded set of stakers.',
          },
        ].map((card) => (
          <Link
            key={card.href}
            href={card.href}
            className="card space-y-2 transition-colors hover:border-neutral-700"
          >
            <h2 className="font-medium">{card.title}</h2>
            <p className="text-sm text-neutral-400">{card.body}</p>
          </Link>
        ))}
      </section>

      {status && (
        <section className="card">
          <h2 className="mb-2 text-sm font-medium">Program</h2>
          <p className="font-mono text-xs text-neutral-400">{status.programId}</p>
          <p className="mt-2 text-xs text-neutral-600">
            Deployed to {status.cluster} · {shortAddress(status.programId)}
          </p>
        </section>
      )}
    </div>
  );
}
