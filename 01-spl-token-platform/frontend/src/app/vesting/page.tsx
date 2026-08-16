'use client';

import {useWallet} from '@solana/wallet-adapter-react';
import {useQuery} from '@tanstack/react-query';
import {api, formatAmount, formatCountdown, shortAddress, type VestingSchedule} from '@/lib/api';
import {cn} from '@/lib/cn';

function ScheduleCard({schedule}: {schedule: VestingSchedule}) {
  const total = BigInt(schedule.totalAmount);
  const released = BigInt(schedule.releasedAmount);
  const vested = BigInt(schedule.vestedAmount);
  const releasable = BigInt(schedule.releasableAmount);

  const releasedPercent = total === 0n ? 0 : Number((released * 10_000n) / total) / 100;
  const vestedPercent = total === 0n ? 0 : Number((vested * 10_000n) / total) / 100;

  const start = new Date(schedule.startTs).getTime();
  const cliffEnd = start + schedule.cliffSeconds * 1000;
  const end = start + schedule.durationSeconds * 1000;
  const beforeCliff = Date.now() < cliffEnd;

  return (
    <div className="card space-y-5">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div>
          <h3 className="font-medium">{shortAddress(schedule.address)}</h3>
          <p className="text-sm text-neutral-500">
            {formatAmount(schedule.totalAmount)} over{' '}
            {Math.round(schedule.durationSeconds / 86_400)} days
            {schedule.cliffSeconds > 0
              ? `, ${Math.round(schedule.cliffSeconds / 86_400)} day cliff`
              : ''}
          </p>
        </div>

        <div className="flex gap-2">
          {schedule.revoked && (
            <span className="rounded-full bg-red-500/15 px-2.5 py-1 text-xs text-red-300">
              Revoked
            </span>
          )}
          {schedule.revocable && !schedule.revoked && (
            <span className="rounded-full bg-amber-500/15 px-2.5 py-1 text-xs text-amber-300">
              Revocable
            </span>
          )}
        </div>
      </div>

      {/* Two-tone bar: released is solid, vested-but-unclaimed is lighter. */}
      <div>
        <div className="relative h-2 overflow-hidden rounded-full bg-neutral-800">
          <div
            className="absolute inset-y-0 left-0 bg-violet-500/40"
            style={{width: `${vestedPercent}%`}}
          />
          <div
            className="absolute inset-y-0 left-0 bg-violet-500"
            style={{width: `${releasedPercent}%`}}
          />
        </div>
        <div className="mt-2 flex justify-between text-xs text-neutral-500">
          <span>{formatAmount(schedule.releasedAmount)} claimed</span>
          <span>{formatAmount(schedule.totalAmount)} total</span>
        </div>
      </div>

      <div className="flex flex-wrap items-center justify-between gap-3 border-t border-neutral-800 pt-4">
        <div>
          <p className="text-xs uppercase tracking-wide text-neutral-500">Claimable now</p>
          <p className="text-xl font-semibold tabular-nums">
            {formatAmount(schedule.releasableAmount)}
          </p>
          {beforeCliff && (
            <p className="mt-1 text-xs text-amber-400">
              Cliff ends in {formatCountdown(cliffEnd) ?? 'now'}
            </p>
          )}
          {!beforeCliff && Date.now() < end && !schedule.revoked && (
            <p className="mt-1 text-xs text-neutral-500">
              Fully vested in {formatCountdown(end) ?? 'now'}
            </p>
          )}
        </div>

        <button
          type="button"
          className={cn('btn-primary', releasable === 0n && 'opacity-50')}
          disabled={releasable === 0n}
        >
          Release
        </button>
      </div>

      {/* Worth stating plainly: releasing is open to anyone, and cannot be redirected. */}
      <p className="text-xs text-neutral-600">
        Anyone can trigger a release, but the destination is constrained on-chain to an account this
        beneficiary owns, so a project can pay the fee on your behalf without touching the tokens.
      </p>

      {schedule.releases.length > 0 && (
        <details className="text-sm">
          <summary className="cursor-pointer text-neutral-500 hover:text-neutral-300">
            {schedule.releases.length} previous release
            {schedule.releases.length === 1 ? '' : 's'}
          </summary>
          <ul className="mt-2 divide-y divide-neutral-800">
            {schedule.releases.map((release) => (
              <li key={release.id} className="flex justify-between py-2">
                <span className="font-mono text-xs text-neutral-500">
                  {shortAddress(release.signature)}
                </span>
                <span className="tabular-nums">{formatAmount(release.amount)}</span>
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

export default function VestingPage() {
  const {publicKey} = useWallet();

  const {data, isLoading} = useQuery({
    queryKey: ['vesting', publicKey?.toBase58()],
    queryFn: () => api.vesting(publicKey!.toBase58()),
    enabled: Boolean(publicKey),
    refetchInterval: 30_000,
  });

  return (
    <div className="space-y-8">
      <header className="space-y-2">
        <h1 className="text-3xl font-semibold tracking-tight">Vesting</h1>
        <p className="max-w-2xl text-neutral-400">
          Each grant lives in its own program-owned vault, funded when the schedule was created.
          Tokens vest linearly from the start, so crossing a cliff unlocks everything accrued during
          it at once.
        </p>
      </header>

      {!publicKey ? (
        <p className="py-24 text-center text-neutral-500">
          Connect a wallet to see your vesting schedules.
        </p>
      ) : isLoading ? (
        <div className="h-64 animate-pulse rounded-xl bg-neutral-900" />
      ) : (data?.schedules.length ?? 0) === 0 ? (
        <p className="py-24 text-center text-neutral-600">No vesting schedules for this wallet.</p>
      ) : (
        <div className="space-y-6">
          {data!.schedules.map((schedule) => (
            <ScheduleCard key={schedule.address} schedule={schedule} />
          ))}
        </div>
      )}
    </div>
  );
}
