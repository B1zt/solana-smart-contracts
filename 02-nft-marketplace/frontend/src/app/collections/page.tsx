'use client';

import {useQuery} from '@tanstack/react-query';
import Link from 'next/link';
import {api, formatSol} from '@/lib/api';

export default function CollectionsPage() {
  const {data, isLoading} = useQuery({
    queryKey: ['collections'],
    queryFn: api.collections,
    refetchInterval: 30_000,
  });

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Collections</h1>
        <p className="max-w-2xl text-neutral-400">
          Listed NFTs are held in program-owned escrow. A listing that leaves the seller in custody
          lets them move the asset mid-listing, and every purchase then fails after the buyer has
          already paid a fee.
        </p>
      </header>

      {isLoading ? (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {Array.from({length: 6}, (_unused, index) => (
            <div key={index} className="h-44 animate-pulse rounded-xl bg-neutral-900" />
          ))}
        </div>
      ) : (data?.collections.length ?? 0) === 0 ? (
        <p className="py-16 text-center text-neutral-600">No collections deployed yet.</p>
      ) : (
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {data!.collections.map((collection) => (
            <Link
              key={collection.address}
              href={`/collections/${collection.address}`}
              className="card space-y-4 transition-colors hover:border-neutral-700"
            >
              <div>
                <h2 className="font-medium">{collection.name}</h2>
                <p className="text-sm text-neutral-500">
                  {collection.minted.toLocaleString()} / {collection.maxSupply.toLocaleString()}{' '}
                  minted
                </p>
              </div>

              <div className="h-1.5 overflow-hidden rounded-full bg-neutral-800">
                <div
                  className="h-full rounded-full bg-violet-500"
                  style={{
                    width: `${Math.min(100, (collection.minted / collection.maxSupply) * 100)}%`,
                  }}
                />
              </div>

              <dl className="flex justify-between text-sm">
                <div>
                  <dt className="text-xs text-neutral-500">Floor</dt>
                  <dd className="tabular-nums">{formatSol(collection.floorPrice)}</dd>
                </div>
                <div className="text-right">
                  <dt className="text-xs text-neutral-500">Royalty</dt>
                  <dd className="tabular-nums">{(collection.royaltyBps / 100).toFixed(1)}%</dd>
                </div>
              </dl>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
