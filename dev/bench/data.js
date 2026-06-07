window.BENCHMARK_DATA = {
  "lastUpdate": 1780865510302,
  "repoUrl": "https://github.com/0xitsss/quince",
  "entries": {
    "QFL Criterion Benchmarks": [
      {
        "commit": {
          "author": {
            "email": "js2302247@gmail.com",
            "name": "0xitsss",
            "username": "0xitsss"
          },
          "committer": {
            "email": "js2302247@gmail.com",
            "name": "0xitsss",
            "username": "0xitsss"
          },
          "distinct": true,
          "id": "a333962f872e39bda0c7ac7a17d1f3a715c5e722",
          "message": "v0.6.9: fix Windows .exe extension in release, restore Cargo.lock before benchmark gh-pages switch",
          "timestamp": "2026-06-08T00:46:24+04:00",
          "tree_id": "7d1d12983cdc5d657793ac7d58e6186fa4f4cb63",
          "url": "https://github.com/0xitsss/quince/commit/a333962f872e39bda0c7ac7a17d1f3a715c5e722"
        },
        "date": 1780865509497,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/31",
            "value": 66525,
            "range": "± 454",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/119",
            "value": 128056,
            "range": "± 1384",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/67",
            "value": 72783,
            "range": "± 2257",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/93",
            "value": 87417,
            "range": "± 658",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 216497,
            "range": "± 1022",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/60",
            "value": 69041,
            "range": "± 340",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/122",
            "value": 121156,
            "range": "± 256",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/152",
            "value": 155436,
            "range": "± 550",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/119",
            "value": 120051,
            "range": "± 219",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/81",
            "value": 101405,
            "range": "± 1083",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 18632,
            "range": "± 60",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/60",
            "value": 67132,
            "range": "± 113",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/52",
            "value": 62238,
            "range": "± 99",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/146",
            "value": 154902,
            "range": "± 7352",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/31",
            "value": 1304543,
            "range": "± 1299",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/119",
            "value": 2212850,
            "range": "± 5992",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/67",
            "value": 852052,
            "range": "± 1758",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/93",
            "value": 2191234,
            "range": "± 15486",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2556326,
            "range": "± 55838",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/60",
            "value": 778376,
            "range": "± 16087",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/122",
            "value": 1898206,
            "range": "± 90066",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/152",
            "value": 3243301,
            "range": "± 10510",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/119",
            "value": 1768238,
            "range": "± 76152",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/81",
            "value": 1171507,
            "range": "± 2047",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 305292,
            "range": "± 351",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/60",
            "value": 771661,
            "range": "± 18733",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/52",
            "value": 835989,
            "range": "± 3660",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/146",
            "value": 935308,
            "range": "± 26741",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 252256,
            "range": "± 517",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2524931,
            "range": "± 4911",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 25576488,
            "range": "± 304088",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2770056,
            "range": "± 8996",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}