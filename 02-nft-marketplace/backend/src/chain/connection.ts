import {Connection} from '@solana/web3.js';
import {config} from '../config.js';

/**
 * Shared RPC connection.
 *
 * `confirmed` rather than `finalized` as the default commitment. Finalised takes roughly 13 seconds
 * on Solana while confirmed takes under a second, and for reading account state that a user is
 * about to act on, confirmed is what makes the interface feel responsive. Anything financial that
 * must not be reversed reads at finalised explicitly.
 */
export const connection = new Connection(config.RPC_URL, {
  commitment: 'confirmed',
  wsEndpoint: config.RPC_WS_URL,
  confirmTransactionInitialTimeout: 60_000,
});

/** Current slot, as a cheap liveness probe for the RPC. */
export async function currentSlot(): Promise<number> {
  return connection.getSlot('confirmed');
}
