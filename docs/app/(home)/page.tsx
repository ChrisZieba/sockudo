import Image from 'next/image';
import Link from 'next/link';
import {
  ArrowRight,
  BellRing,
  Blocks,
  BookOpen,
  Bot,
  CheckCircle2,
  ChevronRight,
  CloudCog,
  Coffee,
  Database,
  GitBranch,
  History,
  Layers3,
  LockKeyhole,
  Network,
  RadioTower,
  Route,
  Server,
  Smartphone,
  Sparkles,
  Terminal,
  Zap,
} from 'lucide-react';

const paths = [
  {
    title: 'Launch the server',
    description: 'Install Sockudo, choose a runtime, configure apps, and get a local realtime endpoint running.',
    href: '/docs/getting-started/installation',
    icon: Terminal,
    meta: 'Start here',
  },
  {
    title: 'Choose a client path',
    description: 'Keep Pusher clients, use the opt-in Ably REST/WebSocket facade, or adopt Protocol V2 SDKs.',
    href: '/docs/reference/compatibility',
    icon: Smartphone,
    meta: 'Compatibility + SDKs',
  },
  {
    title: 'Operate the cluster',
    description: 'Plan scaling, metrics, webhooks, rate limits, queues, durable history, and recovery paths.',
    href: '/docs/server/scaling',
    icon: CloudCog,
    meta: 'Production ops',
  },
  {
    title: 'Build AI transport',
    description: 'Layer agent streams, rollups, push, annotations, and recovery on the same durable primitives.',
    href: '/docs/server/ai-transport-overview',
    icon: Bot,
    meta: 'AI native',
  },
];

const capabilities = [
  {
    title: 'Pusher-compatible edge',
    description: 'Protocol V1 preserves Channels clients, Laravel Echo, auth shapes, and server publish flows.',
    icon: Route,
  },
  {
    title: 'Ably compatibility facade',
    description: 'Opt-in REST and WebSocket compatibility over JSON and MessagePack, excluding Live Objects.',
    icon: RadioTower,
  },
  {
    title: 'Protocol V2 control',
    description: 'Native prefixes, message IDs, serial continuity, filters, deltas, annotations, and rewind.',
    icon: GitBranch,
  },
  {
    title: 'Horizontal fanout',
    description: 'Redis, Redis Cluster, NATS, Kafka, RabbitMQ, Pulsar, Google Pub/Sub, Iggy, or memory.',
    icon: Blocks,
  },
  {
    title: 'Durable recovery',
    description: 'Hot replay buffers, durable history, opaque cursors, presence history, and mutable messages.',
    icon: History,
  },
  {
    title: 'Push and operations',
    description: 'Push, webhooks, metrics, readiness, quotas, retries, provider status, and delivery signals.',
    icon: BellRing,
  },
];

const compatibilityPaths = [
  {
    title: 'Pusher clients',
    protocol: 'Protocol V1',
    detail: 'pusher-js · Laravel Echo',
    icon: Route,
  },
  {
    title: 'Ably REST + WebSocket',
    protocol: 'Opt-in compatibility',
    detail: 'JSON · MessagePack · no Live Objects',
    icon: RadioTower,
  },
  {
    title: 'Sockudo SDKs',
    protocol: 'Protocol V2',
    detail: 'Recovery · AI Transport',
    icon: GitBranch,
  },
];

const sdkCards = [
  ['JavaScript', '@sockudo/client', '/docs/clients/javascript'],
  ['Swift', 'SockudoSwift', '/docs/clients/swift'],
  ['Kotlin', 'io.sockudo:sockudo-kotlin', '/docs/clients/kotlin'],
  ['Flutter', 'sockudo_flutter', '/docs/clients/flutter'],
  ['.NET realtime', 'Sockudo.Client', '/docs/clients/dotnet'],
  ['Node HTTP', 'sockudo', '/docs/server-sdks/node'],
  ['Python HTTP', 'sockudo-http-python', '/docs/server-sdks/python'],
  ['Rust HTTP', 'sockudo-http', '/docs/server-sdks/rust'],
];

const blogPosts = [
  {
    title: 'Designing a realtime protocol that can evolve',
    href: '/blog/protocol-v2-design',
    meta: 'Architecture',
    description:
      'How Sockudo preserves Pusher compatibility while adding serial continuity, native features, and migration room.',
  },
  {
    title: 'Running realtime infrastructure across nodes',
    href: '/blog/horizontal-realtime-operations',
    meta: 'Operations',
    description:
      'Adapters, duplicate delivery, recovery, observability, and fanout failure modes for production clusters.',
  },
  {
    title: 'Choosing SDK surfaces for product teams',
    href: '/blog/sdk-surface-area',
    meta: 'SDKs',
    description:
      'Where credentials belong, how client and server responsibilities divide, and which SDK surface to reach for.',
  },
];

export default function HomePage() {
  return (
    <div className="home-page">
      <section className="home-hero" aria-labelledby="home-hero-title">
        <div className="home-container home-hero-grid">
          <div className="home-hero-copy">
            <div className="home-eyebrow-row">
              <span className="home-badge">
                <Image
                  src="/sockudo-logo/sockudo-icon-color.svg"
                  alt=""
                  width={20}
                  height={20}
                  style={{ width: 20, height: 20 }}
                  aria-hidden="true"
                />
                Open-source realtime infrastructure
              </span>
              <Link
                className="home-support-link"
                href="https://buymeacoffee.com/radooku"
                rel="noreferrer"
                target="_blank"
              >
                <Coffee className="size-4" />
                Support the project
              </Link>
            </div>

            <h1 id="home-hero-title">
              Own realtime infrastructure. Keep the clients you already ship.
            </h1>
            <p className="home-lede">
              Sockudo is a self-hosted Rust realtime server. Keep Pusher clients on Protocol V1, or
              opt into Ably REST and WebSocket compatibility, excluding Live Objects. Move to
              Protocol V2 durability, horizontal fanout, push, and AI Transport when you need them.
            </p>

            <div className="home-actions" aria-label="Primary documentation links">
              <Link className="home-button home-button-primary" href="/docs/getting-started/installation">
                Start building
                <ArrowRight className="size-4" />
              </Link>
              <Link className="home-button home-button-secondary" href="/docs/getting-started/migration">
                Migration guide
                <Zap className="size-4" />
              </Link>
              <Link
                className="home-button home-button-ghost"
                href="/docs/server/ably-ai-transport-compatibility"
              >
                Ably compatibility
              </Link>
            </div>
          </div>

          <div
            className="home-compatibility-map"
            aria-label="Pusher, Ably, and native Sockudo client paths converging on one self-hosted server"
          >
            <div className="compatibility-header">
              <div>
                <span>Client compatibility</span>
                <strong>Choose the edge. Keep control of the server.</strong>
              </div>
              <Link href="/docs/reference/compatibility">
                View scope
                <ArrowRight className="size-4" />
              </Link>
            </div>

            <div className="compatibility-routes">
              {compatibilityPaths.map((path) => (
                <article className="compatibility-route" key={path.title}>
                  <span className="compatibility-route-icon">
                    <path.icon className="size-5" />
                  </span>
                  <span className="compatibility-route-protocol">{path.protocol}</span>
                  <strong>{path.title}</strong>
                  <small>{path.detail}</small>
                </article>
              ))}
            </div>

            <div className="compatibility-merge" aria-hidden="true">
              <span />
              <em>one self-hosted edge</em>
              <span />
            </div>

            <div className="compatibility-server">
              <span className="compatibility-server-logo">
                <Image
                  src="/sockudo-logo/sockudo-icon-color.svg"
                  alt=""
                  width={52}
                  height={52}
                  style={{ width: 52, height: 52 }}
                />
              </span>
              <div>
                <span>Deployed in your infrastructure</span>
                <strong>Sockudo</strong>
                <small>Rust + Tokio · one server, multiple protocol edges</small>
              </div>
              <span className="compatibility-owned">
                <CheckCircle2 className="size-4" />
                Open source
              </span>
            </div>

            <div className="compatibility-outcomes" aria-label="Native Sockudo capabilities">
              <span>Horizontal fanout</span>
              <span>Durable recovery</span>
              <span>Push + AI Transport</span>
            </div>
          </div>
        </div>
      </section>

      <section className="home-section home-section-soft" aria-labelledby="home-paths-title">
        <div className="home-container">
          <div className="home-section-heading">
            <span className="home-badge home-badge-muted">
              <BookOpen className="size-4" />
              Documentation map
            </span>
            <h2 id="home-paths-title">Start with the decision you are making.</h2>
            <p>
              The homepage routes operators, backend teams, client developers, and AI product teams
              into the part of the docs that answers their next production question.
            </p>
          </div>

          <div className="home-path-grid">
            {paths.map((path) => (
              <Link className="home-card home-path-card" href={path.href} key={path.title}>
                <span className="home-card-icon">
                  <path.icon className="size-5" />
                </span>
                <span className="home-card-meta">{path.meta}</span>
                <h3>{path.title}</h3>
                <p>{path.description}</p>
                <span className="home-card-action">
                  Open guide
                  <ChevronRight className="size-4" />
                </span>
              </Link>
            ))}
          </div>
        </div>
      </section>

      <section className="home-section" aria-labelledby="home-architecture-title">
        <div className="home-container home-architecture-grid">
          <div className="home-section-heading home-section-heading-sticky">
            <span className="home-badge home-badge-muted">
              <Network className="size-4" />
              Runtime architecture
            </span>
            <h2 id="home-architecture-title">A compatibility layer that grows into a durable platform.</h2>
            <p>
              Sockudo keeps the Protocol V1 contract stable while Protocol V2 unlocks history,
              recovery, annotations, push delivery, and AI stream primitives on the same fanout core.
            </p>
            <Link className="home-inline-link" href="/docs/server/history-recovery">
              Read history and recovery
              <ArrowRight className="size-4" />
            </Link>
          </div>

          <div className="home-architecture-panel">
            <Image
              src="/diagrams/architecture.svg"
              alt="Sockudo architecture diagram showing clients, adapters, queues, cache, app manager, metrics, webhooks, and storage."
              width={760}
              height={500}
              className="architecture-image"
            />
          </div>
        </div>
      </section>

      <section className="home-section home-section-ink" aria-labelledby="home-capabilities-title">
        <div className="home-container">
          <div className="home-section-heading home-section-heading-invert">
            <span className="home-badge home-badge-dark">
              <Layers3 className="size-4" />
              Production surface
            </span>
            <h2 id="home-capabilities-title">Everything operators expect before realtime becomes critical path.</h2>
            <p>
              Compatibility is the on-ramp. The docs also cover the stateful, distributed, and
              observable pieces that make Sockudo viable when missed events are not acceptable.
            </p>
          </div>

          <div className="home-feature-grid">
            {capabilities.map((feature) => (
              <article className="home-card home-feature-card" key={feature.title}>
                <span className="home-card-icon">
                  <feature.icon className="size-5" />
                </span>
                <h3>{feature.title}</h3>
                <p>{feature.description}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="home-section home-section-soft" aria-labelledby="home-sdk-title">
        <div className="home-container home-sdk-layout">
          <div className="home-section-heading">
            <span className="home-badge home-badge-muted">
              <Server className="size-4" />
              SDK shelf
            </span>
            <h2 id="home-sdk-title">Client and server SDKs stay close to the protocol.</h2>
            <p>
              Every official SDK path includes installation, configuration, authentication, publish
              flows, history, and production guidance for teams adopting Sockudo incrementally.
            </p>
          </div>

          <div className="home-sdk-grid">
            {sdkCards.map(([name, packageName, href]) => (
              <Link className="home-sdk-card" href={href} key={name}>
                <span>{packageName}</span>
                <h3>{name}</h3>
                <p>
                  Open guide
                  <ArrowRight className="size-4" />
                </p>
              </Link>
            ))}
          </div>
        </div>
      </section>

      <section className="home-section" aria-labelledby="home-notes-title">
        <div className="home-container">
          <div className="home-notes-header">
            <div className="home-section-heading">
              <span className="home-badge home-badge-muted">
                <Sparkles className="size-4" />
                Engineering notes
              </span>
              <h2 id="home-notes-title">Design notes for the realtime edge.</h2>
              <p>
                The blog explains the tradeoffs behind the docs: protocol evolution, multi-node
                operations, SDK boundaries, and the practical shape of ownership.
              </p>
            </div>
            <div className="home-security-panel" aria-label="Production readiness highlights">
              <div>
                <LockKeyhole className="size-5" />
                <span>Auth, origin checks, TLS, quotas</span>
              </div>
              <div>
                <Database className="size-5" />
                <span>Durable history and version stores</span>
              </div>
              <div>
                <CheckCircle2 className="size-5" />
                <span>Metrics, health, readiness, webhooks</span>
              </div>
            </div>
          </div>

          <div className="home-blog-grid">
            {blogPosts.map((post) => (
              <Link className="home-card home-blog-card" href={post.href} key={post.href}>
                <span className="home-card-meta">{post.meta}</span>
                <h3>{post.title}</h3>
                <p>{post.description}</p>
              </Link>
            ))}
          </div>
        </div>
      </section>

    </div>
  );
}
