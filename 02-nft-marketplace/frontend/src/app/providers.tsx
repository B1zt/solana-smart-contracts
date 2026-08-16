'use client';

import '@solana/wallet-adapter-react-ui/styles.css';
import {ConnectionProvider, WalletProvider} from '@solana/wallet-adapter-react';
import {WalletModalProvider} from '@solana/wallet-adapter-react-ui';
import {QueryClient, QueryClientProvider} from '@tanstack/react-query';
import {useMemo, useState, type ReactNode} from 'react';

export function Providers({children}: {children: ReactNode}) {
  // Created inside a state initialiser rather than at module scope. A module-level client would be
  // shared across every request during SSR, leaking one user's cached data into another's render.
  const [queryClient] = useState(
    () =>
      new QueryClient({
        defaultOptions: {
          queries: {
            staleTime: 10_000,
            refetchOnWindowFocus: true,
            retry: 1,
          },
        },
      }),
  );

  const endpoint = process.env.NEXT_PUBLIC_RPC_URL ?? 'https://api.devnet.solana.com';

  /**
   * An empty wallet list is deliberate.
   *
   * Every current Solana wallet implements the Wallet Standard, which the adapter discovers
   * automatically at runtime. Listing adapters explicitly, as older tutorials do, both duplicates
   * that discovery and ships a bundle of adapter code for wallets the user does not have.
   */
  const wallets = useMemo(() => [], []);

  return (
    <ConnectionProvider endpoint={endpoint} config={{commitment: 'confirmed'}}>
      <WalletProvider wallets={wallets} autoConnect>
        <WalletModalProvider>
          <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
        </WalletModalProvider>
      </WalletProvider>
    </ConnectionProvider>
  );
}
