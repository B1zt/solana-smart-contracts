'use client';

import {useQuery} from '@tanstack/react-query';
import {use, useState} from 'react';
import {api, formatSol} from '@/lib/api';
import {cn} from '@/lib/cn';
import {formatRelativeTime, shortAddress} from '@/lib/format';

export default function CollectionPage({params}: {params: Promise<{address: string}>}) {
  const {address} = use(params);
  const [tab, setTab] = useState<'items' | 'activity'>('items');

  const {data} = useQuery({
    queryKey: ['collection', address],
    queryFn: () => api.collection(address),
  });

  const {data: listings, isLoading} = useQuery({
    queryKey: ['listings', address],
    queryFn: () => api.listings(address),
    refetchInterval: 20_000,
  });

  const {data: activity} = useQuery({
    queryKey: ['activity', address],
    queryFn: () => api.activity(address),
    enabled: tab === 'activity',
  });

  const collection = data?.collection;

  return (
    <div className="space-y-8">
      <header className="space-y-4">
        <h1 className="text-3xl font-semibold tracking-tight">
          {collection?.name ?? shortAddress(address)}
        </h1>

        <dl className="flex flex-wrap gap-x-8 gap-y-2 text-sm">
          <div>
            <dt className="text-neutral-500">Floor</dt>
            <dd className="font-medium tabular-nums">{formatSol(collection?.floorPrice)}</dd>
          </div>
          <div>
            <dt className="text-neutral-500">Listed</dt>
            <dd className="font-medium tabular-nums">{data?.listedCount ?? '-'}</dd>
          </div>
          <div>
            <dt className="text-neutral-500">Sales</dt>
            <dd className="font-medium tabular-nums">{data?.salesCount ?? '-'}</dd>
          </div>
          <div>
            <dt className="text-neutral-500">Supply</dt>
            <dd className="font-medium tabular-nums">
              {collection ? `${collection.minted} / ${collection.maxSupply}` : '-'}
            </dd>
          </div>
        </dl>
      </header>

      <div className="flex gap-1 border-b border-neutral-800">
        {(['items', 'activity'] as const).map((value) => (
          <button
            key={value}
            type="button"
            onClick={() => setTab(value)}
            className={cn(
              '-mb-px border-b-2 px-4 py-2.5 text-sm font-medium capitalize transition-colors',
              tab === value
                ? 'border-violet-500 text-white'
                : 'border-transparent text-neutral-500 hover:text-neutral-300',
            )}
          >
            {value}
          </button>
        ))}
      </div>

      {tab === 'items' ? (
        isLoading ? (
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-4">
            {Array.from({length: 8}, (_unused, index) => (
              <div key={index} className="aspect-square animate-pulse rounded-xl bg-neutral-900" />
            ))}
          </div>
        ) : (listings?.listings.length ?? 0) === 0 ? (
          <p className="py-16 text-center text-neutral-600">Nothing listed right now.</p>
        ) : (
          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-4">
            {listings!.listings.map((listing) => (
              <div
                key={listing.address}
                className="overflow-hidden rounded-xl border border-neutral-800 bg-neutral-900/50 transition-colors hover:border-neutral-700"
              >
                <div className="aspect-square bg-neutral-900">
                  {listing.image ? (
                    // Plain img: asset art comes from arbitrary gateways whose hosts cannot be
                    // allowlisted for the image optimiser at build time.
                    <img
                      src={listing.image}
                      alt={listing.name ?? shortAddress(listing.mint)}
                      loading="lazy"
                      className="h-full w-full object-cover"
                    />
                  ) : (
                    <div className="flex h-full items-center justify-center text-xs text-neutral-600">
                      No image
                    </div>
                  )}
                </div>

                <div className="space-y-1 p-3">
                  <div className="flex items-center justify-between gap-2">
                    <p className="truncate text-sm font-medium">
                      {listing.name ?? shortAddress(listing.mint)}
                    </p>
                    {listing.compressed && (
                      <span
                        className="shrink-0 rounded bg-violet-500/15 px-1.5 py-0.5 text-[10px] text-violet-300"
                        title="Compressed NFT: stored as a Merkle leaf rather than an account"
                      >
                        cNFT
                      </span>
                    )}
                  </div>
                  <p className="text-sm tabular-nums text-neutral-300">
                    {formatSol(listing.price)}
                  </p>
                </div>
              </div>
            ))}
          </div>
        )
      ) : (
        <div className="overflow-x-auto rounded-xl border border-neutral-800">
          <table className="w-full text-sm">
            <thead className="bg-neutral-900/50 text-left text-xs uppercase tracking-wide text-neutral-500">
              <tr>
                <th className="px-4 py-3">Item</th>
                <th className="px-4 py-3">Price</th>
                <th className="px-4 py-3">From</th>
                <th className="px-4 py-3">To</th>
                <th className="px-4 py-3">When</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-neutral-800">
              {(activity?.sales ?? []).map((sale) => (
                <tr key={sale.id} className="hover:bg-neutral-900/40">
                  <td className="px-4 py-3 font-mono text-xs">{shortAddress(sale.mint)}</td>
                  <td className="px-4 py-3 tabular-nums">{formatSol(sale.price)}</td>
                  <td className="px-4 py-3 text-neutral-400">{shortAddress(sale.seller)}</td>
                  <td className="px-4 py-3 text-neutral-400">{shortAddress(sale.buyer)}</td>
                  <td className="px-4 py-3 text-neutral-500">
                    {formatRelativeTime(sale.blockTime)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>

          {(activity?.sales.length ?? 0) === 0 && (
            <p className="py-16 text-center text-neutral-600">No sales yet.</p>
          )}
        </div>
      )}
    </div>
  );
}
