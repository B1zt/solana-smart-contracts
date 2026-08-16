import cors from '@fastify/cors';
import rateLimit from '@fastify/rate-limit';
import Fastify, {type FastifyError, type FastifyInstance} from 'fastify';
import {config} from './config.js';
import {loggerOptions} from './lib/logger.js';
import {prisma} from './lib/prisma.js';
import {apiRoutes} from './routes.js';

export async function buildServer(): Promise<FastifyInstance> {
  const app = Fastify({
    logger: loggerOptions,
    trustProxy: true,
    bodyLimit: 8 * 1024 * 1024,
  });

  await app.register(cors, {origin: config.CORS_ORIGINS, methods: ['GET', 'POST']});
  await app.register(rateLimit, {max: 300, timeWindow: '1 minute'});

  app.get('/health', async () => {
    await prisma.$queryRaw`SELECT 1`;
    return {status: 'ok', cluster: config.CLUSTER};
  });

  await app.register(apiRoutes, {prefix: '/api/v1'});

  app.setErrorHandler((error: FastifyError, request, reply) => {
    request.log.error({error}, 'request failed');

    const status = error.statusCode ?? 500;
    return reply.status(status).send({error: status >= 500 ? 'Internal server error' : error.message});
  });

  return app;
}
