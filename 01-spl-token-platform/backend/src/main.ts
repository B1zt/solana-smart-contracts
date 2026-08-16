import {config} from './config.js';
import {Indexer} from './indexer/indexer.js';
import {logger} from './lib/logger.js';
import {prisma} from './lib/prisma.js';
import {buildServer} from './server.js';

async function main(): Promise<void> {
  const app = await buildServer();
  const indexer = new Indexer();

  await app.listen({port: config.PORT, host: config.HOST});
  logger.info({port: config.PORT, cluster: config.CLUSTER}, 'api listening');

  if (config.INDEXER_ENABLED) {
    await indexer.start();
  } else {
    logger.warn('indexer disabled; the API is serving whatever is already in the database');
  }

  const shutdown = async (signal: string): Promise<void> => {
    logger.info({signal}, 'shutting down');
    indexer.stop();
    await app.close();
    await prisma.$disconnect();
    process.exit(0);
  };

  process.on('SIGTERM', () => void shutdown('SIGTERM'));
  process.on('SIGINT', () => void shutdown('SIGINT'));
}

main().catch((error) => {
  logger.fatal({error}, 'failed to start');
  process.exit(1);
});
