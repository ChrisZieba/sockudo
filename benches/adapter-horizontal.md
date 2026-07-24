# Horizontal adapter benchmark

This benchmark compares Redis and Apache Iggy fanout through three Sockudo
nodes. It uses the production release profile (`lto = true`,
`codegen-units = 1`) and the same Sockudo image for both adapters.

## Workload

- 3 Sockudo nodes
- 900 WebSocket subscribers
- 100 public channels
- 3 subscribers per channel on each node (9 deliveries per publish)
- HTTP publishes distributed evenly across all nodes
- 10-second subscriber warmup
- 20-second measured publish window
- 10-12-second fanout drain
- 100-message Iggy poll batches

The Compose stack pins Redis and Iggy image digests. Run one rate against a
fresh stack so disconnect-time cluster requests from a previous run cannot
contaminate the next measurement:

```bash
docker build \
  --build-arg SOCKUDO_FEATURES=redis,iggy \
  -t sockudo-adapter-benchmark:local .

docker compose \
  -f docker-compose.adapter-benchmark.yml \
  --profile iggy \
  down -v --remove-orphans

ADAPTER_DRIVER=iggy docker compose \
  -f docker-compose.adapter-benchmark.yml \
  --profile iggy \
  up -d --wait

SUBSCRIBERS=900 \
CHANNELS=100 \
MESSAGES_PER_SECOND=5000 \
WARMUP_SECONDS=10 \
DURATION_SECONDS=20 \
DRAIN_SECONDS=12 \
RESULT_PATH=/tmp/iggy-5000.json \
k6 run --quiet benches/adapter-horizontal.js
```

Use `--profile redis` and `ADAPTER_DRIVER=redis` for Redis. Override
`IGGY_POLL_INTERVAL_MS` or `IGGY_POLL_BATCH_SIZE` on the Compose command to
test other Iggy settings.

## Iggy optimization results

Measured on Docker Desktop for Apple Silicon with 18 CPUs and an 8 GiB Docker
memory limit. Iggy used one partition, client 0.10.0, and the pinned 0.8.0
server image.

| Version / rate | Successful publishes | Dropped iterations | Delivery ratio | Delivery p95 | Delivery p99 |
| --- | ---: | ---: | ---: | ---: | ---: |
| Baseline, 1,000 msg/s, 50 ms polls | 20,003 | 0 | 100% | 49 ms | 51 ms |
| Optimized, 1,000 msg/s, 5 ms polls | 20,001 | 0 | 100% | 6 ms | 7 ms |
| Baseline, 4,000 msg/s, 50 ms polls | 78,399 | 1,602 | 81.73% | 14,343 ms | 15,752 ms |
| Optimized, 4,000 msg/s, 5 ms polls | 80,001 | 0 | 100% | 11 ms | 19 ms |
| Optimized, 5,000 msg/s, 5 ms polls | 100,000 | 0 | 100% | 39 ms | 56 ms |
| Optimized, 6,000 msg/s, 5 ms polls | 110,420 | 9,257 | 100% of accepted publishes | 3,001 ms | 3,499 ms |

The stable knee on this host is 5,000 publishes/s, producing 45,000 client
deliveries/s. At 6,000 publishes/s ingress queues and the load generator cannot
maintain the requested arrival rate.

## Redis comparison

The final Redis and optimized Iggy runs used fresh stacks and the same Sockudo
release image:

| Rate | Adapter | Successful publishes | Delivery ratio | Delivery p95 | Delivery p99 |
| --- | --- | ---: | ---: | ---: | ---: |
| 1,000 msg/s | Redis | 20,001 | 100% | 1 ms | 4 ms |
| 1,000 msg/s | Iggy | 20,001 | 100% | 6 ms | 7 ms |
| 5,000 msg/s | Redis | 100,001 | 100% | 35 ms | 46 ms |
| 5,000 msg/s | Iggy | 100,000 | 100% | 39 ms | 56 ms |

At light load Redis retains lower latency because pub/sub pushes messages
immediately, while Iggy polls and persists an ordered log. At the 5,000 msg/s
stable throughput point, their tail latency is within 10 ms in this workload.

The improvement comes from reducing the poll interval from 50 ms to 5 ms and
committing only the highest completed offset per partition after each polled
batch. The latter replaces as many as 100 synchronous offset-store requests
with one request for a one-partition, 100-message batch.
