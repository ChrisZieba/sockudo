import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  allowedDevOrigins: ['127.0.0.1'],
  async redirects() {
    return [
      {
        source: '/install.sh',
        destination: 'https://github.com/sockudo/sockudo/releases/latest/download/install.sh',
        permanent: false,
      },
    ];
  },
};

export default withMDX(config);
