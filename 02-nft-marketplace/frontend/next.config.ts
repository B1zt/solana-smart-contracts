import type {NextConfig} from 'next';

const nextConfig: NextConfig = {
  reactStrictMode: true,

  webpack: (config) => {
    // The Solana wallet adapters reach for Node built-ins that do not exist in a browser bundle.
    // These packages are optional inside the adapters, so resolving them to nothing is correct
    // rather than a workaround; pulling in polyfills would ship dead weight.
    config.resolve = config.resolve ?? {};
    config.resolve.fallback = {
      ...config.resolve.fallback,
      fs: false,
      net: false,
      tls: false,
crypto: false,
    };
    return config;
  },
};

export default nextConfig;
