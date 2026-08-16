'use client';

import {WalletMultiButton} from '@solana/wallet-adapter-react-ui';
import Link from 'next/link';
import {usePathname} from 'next/navigation';
import {cn} from '@/lib/cn';

const links = [
  {href: '/claim', label: 'Airdrop'},
  {href: '/vesting', label: 'Vesting'},
  {href: '/stake', label: 'Stake'},
];

export function Header() {
  const pathname = usePathname();

  return (
    <header className="sticky top-0 z-40 border-b border-neutral-800 bg-neutral-950/80 backdrop-blur">
      <div className="mx-auto flex w-full max-w-6xl items-center justify-between gap-4 px-4 py-4">
        <div className="flex items-center gap-8">
          <Link href="/" className="text-lg font-semibold tracking-tight">
            B1zt<span className="text-violet-400">.sol</span>
          </Link>

          <nav className="hidden items-center gap-1 md:flex">
            {links.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                className={cn(
                  'rounded-md px-3 py-2 text-sm transition-colors',
                  pathname.startsWith(link.href)
                    ? 'bg-neutral-800 text-white'
                    : 'text-neutral-400 hover:bg-neutral-900 hover:text-neutral-100',
                )}
              >
                {link.label}
              </Link>
            ))}
          </nav>
        </div>

        <WalletMultiButton />
      </div>
    </header>
  );
}
