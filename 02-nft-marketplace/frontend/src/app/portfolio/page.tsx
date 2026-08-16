'use client';

import {useWallet} from '@solana/wallet-adapter-react';
import {useQuery} from '@tanstack/react-query';
import {api, ApiError, formatSol} from '@/lib/api';
import {shortAddress} from '@/lib/format';

export default function PortfolioPage() {
  const {publicKey} = useWallet();

  const {data, isLoading, error} = useQuery({
    queryKey: ['walletAssets', publicKey?.toBase58()],
    queryFn: () => api.walletAssets(publicKey!.toBase58()),
    enabled: Boolean(publicKey),
    retry: false,
  });

  // A DAS-less endpoint is a configuration problem with a specific fix, so it gets a specific
  // message rather than a generic failure.
  const dasUnsupported = error instanceof ApiError && error.status === 503;

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Portfolio</h1>
        <p className="max-w-2xl text-neutral-400">
          Holdings come from the Digital Asset Standard rather than a token-account scan, because a
          token-account scan misses compressed NFTs entirely.
        </p>
      </header>

      {!publicKey ? (
        <p className="py-24 text-center text-neutral-500">Connect a wallet to see your NFTs.</p>
      ) : dasUnsupported ? (
        <div className="card border-amber-900 bg-amber-950/20">
          <h2 className="font-medium text-amber-200">This RPC does not support DAS</h2>
          <p className="mt-2 text-sm text-amber-100/70">
            Compressed NFTs have no on-chain account: they are Merkle leaves, with only the root
            stored. Reading them requires a DAS-capable endpoint such as Helius, Triton or
            QuickNode. The public Solana endpoints do not implement it.
          </p>
          <p className="mt-2 text-xs text-amber-100/50">
            Set <code>DAS_RPC_URL</code> in the backend environment.
          </p>
        </div>
      ) : isLoading ? (
        <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5">
          {Array.from({length: 10}, (_unused, index) => (
            <div key={index} className="aspect-square animate-pulse rounded-xl bg-neutral-900" />
          ))}
        </div>
      ) : (data?.assets.length ?? 0) === 0 ? (
        <p className="py-24 text-center text-neutral-600">No NFTs in this wallet.</p>
      ) : (
        <>
          <p className="text-sm text-neutral-500">
            {data!.total} asset{data!.total === 1 ? '' : 's'} ·{' '}
            {data!.assets.filter((asset) => asset.compressed).length} compressed
          </p>

          <div className="grid grid-cols-2 gap-4 sm:grid-cols-3 xl:grid-cols-5">
            {data!.assets.map((asset) => (
              <div
                key={asset.id}
                className="overflow-hidden rounded-xl border border-neutral-800 bg-neutral-900/50"
              >
                <div className="aspect-square bg-neutral-900">
                  {asset.image ? (
                    <img
                      src={asset.image}
                      alt={asset.name ?? shortAddress(asset.id)}
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
                    <p className="truncate text-sm">{asset.name ?? shortAddress(asset.id)}</p>
                    {asset.compressed && (
                      <span className="shrink-0 rounded bg-violet-500/15 px-1.5 py-0.5 text-[10px] text-violet-300">
                        cNFT
                      </span>
                    )}
                  </div>

                  {asset.listing ? (
                    <p className="text-xs tabular-nums text-emerald-400">
                      Listed at {formatSol(asset.listing.price)}
                    </p>
                  ) : (
                    <p className="text-xs text-neutral-600">Not listed</p>
                  )}
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
