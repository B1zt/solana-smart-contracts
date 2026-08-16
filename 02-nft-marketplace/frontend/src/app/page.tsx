'use client';

import {useQuery} from '@tanstack/react-query';
import Link from 'next/link';
import {useState} from 'react';
import {api, formatSol} from '@/lib/api';

/**
 * The compression cost comparison is the landing page's centrepiece.
 *
 * It is the single most persuasive argument for choosing Solana for a large collection, and a live
 * number is more convincing than a claim in a paragraph.
 */
function CompressionCalculator() {
  const [count, setCount] = useState(10_000);

  const {data} = useQuery({
    queryKey: ['compressionSavings', count],
    queryFn: () => api.compressionSavings(count),
  });

  return (
    <div className="card space-y-4">
      <h2 className="font-medium">What compression actually saves</h2>

      <div>
        <label className="label" htmlFor="count">
          Collection size
        </label>
        <input
          id="count"
          type="range"
          min={100}
          max={1_000_000}
          step={100}
          value={count}
          onChange={(event) => setCount(Number(event.target.value))}
          className="w-full accent-violet-500"
        />
        <p className="mt-1 text-sm tabular-nums text-neutral-400">
          {count.toLocaleString()} NFTs
        </p>
      </div>

      {data && (
        <dl className="grid grid-cols-3 gap-4 border-t border-neutral-800 pt-4 text-sm">
          <div>
            <dt className="text-xs uppercase tracking-wide text-neutral-500">Regular</dt>
            <dd className="mt-1 text-lg font-semibold tabular-nums">
              {data.regularSol.toFixed(2)} SOL
            </dd>
          </div>
          <div>
            <dt className="text-xs uppercase tracking-wide text-neutral-500">Compressed</dt>
            <dd className="mt-1 text-lg font-semibold tabular-nums text-emerald-400">
              {data.compressedSol.toFixed(4)} SOL
            </dd>
          </div>
          <div>
            <dt className="text-xs uppercase tracking-wide text-neutral-500">Cheaper by</dt>
            <dd className="mt-1 text-lg font-semibold tabular-nums text-violet-400">
              {Math.round(data.savingsMultiple).toLocaleString()}×
            </dd>
          </div>
        </dl>
      )}

      <p className="text-xs text-neutral-600">
        A regular NFT needs a mint, a token account and a metadata account, roughly 0.012 SOL of
        rent each. A compressed one is a leaf in a shared Merkle tree, so a million of them fit in a
        single account costing around 8 SOL. The trade-off is that transfers need a Merkle proof
        fetched from an indexer.
      </p>
    </div>
  );
}

export default function HomePage() {
  const {data: config} = useQuery({queryKey: ['config'], queryFn: api.config, retry: false});
  const {data: collections} = useQuery({
    queryKey: ['collections'],
    queryFn: api.collections,
    retry: false,
  });

  const floors = (collections?.collections ?? [])
    .map((collection) => collection.floorPrice)
    .filter((price): price is string => price !== null)
    .map((price) => BigInt(price));

  const lowestFloor = floors.length > 0 ? floors.reduce((a, b) => (a < b ? a : b)) : null;

  return (
    <div className="space-y-12">
      <section className="space-y-3">
        <h1 className="text-4xl font-semibold tracking-tight">NFTs on Solana</h1>
        <p className="max-w-2xl text-neutral-400">
          Collection minting with Merkle allowlist phases, and a marketplace where listed assets sit
          in program-owned escrow. Built around Solana&apos;s account model rather than translated
          from Ethereum.
        </p>
      </section>

      <section className="grid gap-4 sm:grid-cols-3">
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">Collections</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">
            {collections?.collections.length ?? '-'}
          </p>
        </div>
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">Lowest floor</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">
            {lowestFloor ? formatSol(lowestFloor) : '-'}
          </p>
        </div>
        <div className="card">
          <p className="text-xs uppercase tracking-wide text-neutral-500">DAS indexer</p>
          <p
            className={
              config?.dasSupported ? 'mt-1 text-2xl font-semibold text-emerald-400' : 'mt-1 text-2xl font-semibold text-amber-400'
            }
          >
            {config === undefined ? '-' : config.dasSupported ? 'Available' : 'Not configured'}
          </p>
        </div>
      </section>

      <CompressionCalculator />

      <section className="grid gap-4 sm:grid-cols-2">
        {[
          {
            href: '/collections',
            title: 'Browse collections',
            body: 'Listed NFTs are escrowed by the program, so a seller cannot move the asset out from under a pending purchase.',
          },
          {
            href: '/portfolio',
            title: 'Your NFTs',
            body: 'Read through the Digital Asset Standard, so compressed NFTs appear alongside regular ones rather than being invisible.',
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

      <section className="card">
        <h2 className="mb-2 text-sm font-medium">On royalties</h2>
        <p className="text-sm text-neutral-400">
          Solana has no protocol-level royalty enforcement. This marketplace reads each
          collection&apos;s royalty setting and pays it on every sale; a different marketplace can
          simply not. That is a property of the chain rather than a gap in this program, and
          creators should understand it before setting expectations.
        </p>
      </section>
    </div>
  );
}
