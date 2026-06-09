window.BENCHMARK_DATA = {
  "lastUpdate": 1780995951415,
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
      },
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
          "id": "706bd7b1fe946cb202623488b638de2780f7b971",
          "message": "docs: add Mermaid UML diagrams to README (architecture, sequence, class, state, pipeline)",
          "timestamp": "2026-06-08T08:12:14+04:00",
          "tree_id": "b3c4347fca6345e872e5df8620acf8f952bfea51",
          "url": "https://github.com/0xitsss/quince/commit/706bd7b1fe946cb202623488b638de2780f7b971"
        },
        "date": 1780892258583,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/31",
            "value": 70440,
            "range": "± 1575",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/119",
            "value": 127681,
            "range": "± 993",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/67",
            "value": 77004,
            "range": "± 155",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/93",
            "value": 86396,
            "range": "± 370",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 219533,
            "range": "± 749",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/60",
            "value": 67886,
            "range": "± 145",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/122",
            "value": 119253,
            "range": "± 386",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/152",
            "value": 152919,
            "range": "± 749",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/119",
            "value": 117953,
            "range": "± 247",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/81",
            "value": 100380,
            "range": "± 786",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 18746,
            "range": "± 30",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/60",
            "value": 67511,
            "range": "± 217",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/52",
            "value": 61455,
            "range": "± 156",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/146",
            "value": 154052,
            "range": "± 564",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/31",
            "value": 1306999,
            "range": "± 741",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/119",
            "value": 2234605,
            "range": "± 10478",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/67",
            "value": 845704,
            "range": "± 1711",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/93",
            "value": 2164496,
            "range": "± 13681",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2541496,
            "range": "± 34641",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/60",
            "value": 815303,
            "range": "± 1973",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/122",
            "value": 1885441,
            "range": "± 7619",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/152",
            "value": 3203028,
            "range": "± 6619",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/119",
            "value": 1701216,
            "range": "± 3564",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/81",
            "value": 1145454,
            "range": "± 3774",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 305363,
            "range": "± 2567",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/60",
            "value": 810045,
            "range": "± 1611",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/52",
            "value": 837063,
            "range": "± 5294",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/146",
            "value": 980107,
            "range": "± 1616",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 255472,
            "range": "± 1302",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2532087,
            "range": "± 34430",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 25340989,
            "range": "± 24130",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2825047,
            "range": "± 35058",
            "unit": "ns/iter"
          }
        ]
      },
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
          "id": "0b8f212d6e2c894712efbcbb07a2793a71434e27",
          "message": "v0.6.10 — bump version",
          "timestamp": "2026-06-09T13:00:37+04:00",
          "tree_id": "45b29b19e01ac1a0c2205c3b22c3f2b2525e2f18",
          "url": "https://github.com/0xitsss/quince/commit/0b8f212d6e2c894712efbcbb07a2793a71434e27"
        },
        "date": 1780995950921,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/31",
            "value": 70429,
            "range": "± 857",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/119",
            "value": 126884,
            "range": "± 2589",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/67",
            "value": 76324,
            "range": "± 515",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/93",
            "value": 92131,
            "range": "± 248",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 218275,
            "range": "± 974",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/60",
            "value": 67712,
            "range": "± 418",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/122",
            "value": 120057,
            "range": "± 342",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/152",
            "value": 156637,
            "range": "± 382",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/119",
            "value": 120951,
            "range": "± 177",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/81",
            "value": 102178,
            "range": "± 271",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 18863,
            "range": "± 143",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/60",
            "value": 67926,
            "range": "± 227",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/52",
            "value": 62364,
            "range": "± 269",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/146",
            "value": 155669,
            "range": "± 1353",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/31",
            "value": 1305858,
            "range": "± 1714",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/119",
            "value": 2104055,
            "range": "± 15200",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/67",
            "value": 837752,
            "range": "± 5741",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/93",
            "value": 2178342,
            "range": "± 10571",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2543570,
            "range": "± 25098",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/60",
            "value": 799888,
            "range": "± 3004",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/122",
            "value": 1893420,
            "range": "± 64595",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/152",
            "value": 3230065,
            "range": "± 10946",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/119",
            "value": 1851591,
            "range": "± 15084",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/81",
            "value": 1144028,
            "range": "± 7048",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 305315,
            "range": "± 315",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/60",
            "value": 800220,
            "range": "± 7518",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/52",
            "value": 832709,
            "range": "± 5802",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/146",
            "value": 981100,
            "range": "± 1940",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 246284,
            "range": "± 1871",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2565743,
            "range": "± 46026",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 25727454,
            "range": "± 245606",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2802629,
            "range": "± 17305",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}