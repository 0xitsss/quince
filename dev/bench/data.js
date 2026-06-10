window.BENCHMARK_DATA = {
  "lastUpdate": 1781097135502,
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
          "id": "47271d8e07907169fe64eec036bb95f582a0bc62",
          "message": "v0.6.10 — bump",
          "timestamp": "2026-06-09T13:01:48+04:00",
          "tree_id": "45b29b19e01ac1a0c2205c3b22c3f2b2525e2f18",
          "url": "https://github.com/0xitsss/quince/commit/47271d8e07907169fe64eec036bb95f582a0bc62"
        },
        "date": 1780996028398,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/31",
            "value": 71860,
            "range": "± 2602",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/119",
            "value": 128362,
            "range": "± 815",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/67",
            "value": 77580,
            "range": "± 276",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/93",
            "value": 93517,
            "range": "± 501",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 221000,
            "range": "± 2341",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/60",
            "value": 68493,
            "range": "± 90",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/122",
            "value": 121765,
            "range": "± 226",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/152",
            "value": 155809,
            "range": "± 608",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/119",
            "value": 119999,
            "range": "± 279",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/81",
            "value": 103108,
            "range": "± 1612",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 18726,
            "range": "± 46",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/60",
            "value": 68766,
            "range": "± 117",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/52",
            "value": 63414,
            "range": "± 206",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/146",
            "value": 162421,
            "range": "± 775",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/31",
            "value": 1306588,
            "range": "± 1067",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/119",
            "value": 2205997,
            "range": "± 15606",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/67",
            "value": 842481,
            "range": "± 3352",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/93",
            "value": 2156068,
            "range": "± 7136",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2536457,
            "range": "± 25984",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/60",
            "value": 806615,
            "range": "± 1773",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/122",
            "value": 1882771,
            "range": "± 10212",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/152",
            "value": 3250807,
            "range": "± 3841",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/119",
            "value": 1970919,
            "range": "± 109081",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/81",
            "value": 1143965,
            "range": "± 18196",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 305232,
            "range": "± 3913",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/60",
            "value": 808032,
            "range": "± 2642",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/52",
            "value": 834876,
            "range": "± 5001",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/146",
            "value": 984874,
            "range": "± 983",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 252565,
            "range": "± 1088",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2524444,
            "range": "± 27037",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 25388198,
            "range": "± 196611",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2780895,
            "range": "± 6179",
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
          "id": "9dee46b5d1ee260cd91df0de859de43ae508d2b9",
          "message": "v0.6.11: add QuinceHash64 checksum + computed_goto VM dispatch\n\n- QuinceHash64: custom ARX sponge 64-bit checksum (256-bit state, 3 finalizing rounds, bit padding + length strengthening)\n- QFRC footer appended on save, verified on load and mmap paths\n- Replace JUMP_TABLE function pointer dispatch with computed_goto! match macro in all 4 VM dispatch loops\n- Remove JUMP_TABLE from opcodes.rs (keep SENTINEL_OPCODE)",
          "timestamp": "2026-06-09T14:07:31+04:00",
          "tree_id": "f12a9f224048cb74adb7e221a550f8a60f76ac4c",
          "url": "https://github.com/0xitsss/quince/commit/9dee46b5d1ee260cd91df0de859de43ae508d2b9"
        },
        "date": 1780999990389,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/31",
            "value": 65914,
            "range": "± 664",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/119",
            "value": 129110,
            "range": "± 972",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/67",
            "value": 76656,
            "range": "± 377",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/93",
            "value": 91214,
            "range": "± 475",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 222039,
            "range": "± 7273",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/60",
            "value": 64869,
            "range": "± 140",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/122",
            "value": 115653,
            "range": "± 397",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/152",
            "value": 156780,
            "range": "± 930",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/119",
            "value": 116485,
            "range": "± 573",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/81",
            "value": 101736,
            "range": "± 1583",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 16763,
            "range": "± 27",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/60",
            "value": 64366,
            "range": "± 811",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/52",
            "value": 62767,
            "range": "± 113",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/146",
            "value": 165999,
            "range": "± 576",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/31",
            "value": 532366,
            "range": "± 479",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/119",
            "value": 1225390,
            "range": "± 1255",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/67",
            "value": 521638,
            "range": "± 415",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/93",
            "value": 1201927,
            "range": "± 800",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 1511130,
            "range": "± 1116",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/60",
            "value": 502079,
            "range": "± 1547",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/122",
            "value": 1105257,
            "range": "± 994",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/152",
            "value": 1848428,
            "range": "± 8268",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/119",
            "value": 1037378,
            "range": "± 5216",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/81",
            "value": 787918,
            "range": "± 471",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 205108,
            "range": "± 291",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/60",
            "value": 500059,
            "range": "± 515",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/52",
            "value": 433351,
            "range": "± 608",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/146",
            "value": 603321,
            "range": "± 398",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 151217,
            "range": "± 97",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 1511765,
            "range": "± 1004",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 15122249,
            "range": "± 43462",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 1595473,
            "range": "± 2550",
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
          "id": "53ec44c956385b37818443e2083282400b6250a2",
          "message": "v0.7.1: fix vm_jmp off-by-one causing infinite loop in compound conditions\n\nFix vm_jmp target computation (was missing +1, causing every Jmp to land 1\ninstruction short — infinite loop when offset was 0). Rewrite BinOp::And/Or\nshort-circuit to eliminate dead Jmp and properly initialize rd.",
          "timestamp": "2026-06-10T16:39:04+04:00",
          "tree_id": "f741889316a68957fed929a0717e6fa7b2fdb12f",
          "url": "https://github.com/0xitsss/quince/commit/53ec44c956385b37818443e2083282400b6250a2"
        },
        "date": 1781095494432,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/51",
            "value": 65353,
            "range": "± 474",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/108",
            "value": 138585,
            "range": "± 1395",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/46",
            "value": 79755,
            "range": "± 883",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/71",
            "value": 97123,
            "range": "± 2550",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 217005,
            "range": "± 3598",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/46",
            "value": 72365,
            "range": "± 330",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/84",
            "value": 137267,
            "range": "± 470",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/142",
            "value": 171111,
            "range": "± 1393",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/80",
            "value": 134246,
            "range": "± 374",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/50",
            "value": 113512,
            "range": "± 1644",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 18479,
            "range": "± 35",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/46",
            "value": 72403,
            "range": "± 126",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/31",
            "value": 70516,
            "range": "± 102",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/123",
            "value": 168594,
            "range": "± 475",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/51",
            "value": 740664,
            "range": "± 2440",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/108",
            "value": 1437635,
            "range": "± 1692",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/46",
            "value": 756806,
            "range": "± 1499",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/71",
            "value": 1014528,
            "range": "± 12126",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2307333,
            "range": "± 15242",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/46",
            "value": 700903,
            "range": "± 1591",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/84",
            "value": 1628933,
            "range": "± 4293",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/142",
            "value": 2484412,
            "range": "± 2945",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/80",
            "value": 1495227,
            "range": "± 4872",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/50",
            "value": 1290173,
            "range": "± 2287",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 243132,
            "range": "± 192",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/46",
            "value": 741654,
            "range": "± 3415",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/31",
            "value": 656864,
            "range": "± 1269",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/123",
            "value": 1368141,
            "range": "± 5453",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 220581,
            "range": "± 1368",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2103475,
            "range": "± 18667",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 22014085,
            "range": "± 68860",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2385965,
            "range": "± 14874",
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
          "id": "9fc19c13ffd981ca0556c48e87d56845004b7c14",
          "message": "fix: deploy book to gh-pages branch (not deploy-pages API)",
          "timestamp": "2026-06-10T17:06:46+04:00",
          "tree_id": "99d78954b101acbca2f5da02bd759fafbc7fc500",
          "url": "https://github.com/0xitsss/quince/commit/9fc19c13ffd981ca0556c48e87d56845004b7c14"
        },
        "date": 1781097135125,
        "tool": "cargo",
        "benches": [
          {
            "name": "pipeline/atr_trail/51",
            "value": 64092,
            "range": "± 718",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/bb_bounce/108",
            "value": 131830,
            "range": "± 653",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/ema_cross/46",
            "value": 74720,
            "range": "± 471",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/grid_trade/71",
            "value": 95410,
            "range": "± 384",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/heavy_test/180",
            "value": 213355,
            "range": "± 4450",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/macd_cross/46",
            "value": 66410,
            "range": "± 134",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/momentum/84",
            "value": 131418,
            "range": "± 467",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rare_signal/142",
            "value": 162116,
            "range": "± 1049",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/rsi_reversion/80",
            "value": 128278,
            "range": "± 3605",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/scalper/50",
            "value": 107336,
            "range": "± 3181",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/simple_test/14",
            "value": 16777,
            "range": "± 77",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/sma_cross/46",
            "value": 65531,
            "range": "± 195",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_all/31",
            "value": 63890,
            "range": "± 218",
            "unit": "ns/iter"
          },
          {
            "name": "pipeline/test_data_passing/123",
            "value": 155928,
            "range": "± 977",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/atr_trail/51",
            "value": 789269,
            "range": "± 459",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/bb_bounce/108",
            "value": 1523987,
            "range": "± 3860",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/ema_cross/46",
            "value": 778703,
            "range": "± 4849",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/grid_trade/71",
            "value": 1099797,
            "range": "± 1581",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/heavy_test/180",
            "value": 2340807,
            "range": "± 9961",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/macd_cross/46",
            "value": 762084,
            "range": "± 1142",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/momentum/84",
            "value": 1728402,
            "range": "± 3286",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rare_signal/142",
            "value": 2391135,
            "range": "± 7385",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/rsi_reversion/80",
            "value": 1635501,
            "range": "± 6463",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/scalper/50",
            "value": 1234196,
            "range": "± 1781",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/simple_test/14",
            "value": 253787,
            "range": "± 349",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/sma_cross/46",
            "value": 762560,
            "range": "± 2554",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_all/31",
            "value": 652213,
            "range": "± 1912",
            "unit": "ns/iter"
          },
          {
            "name": "vm_tick/test_data_passing/123",
            "value": 1514154,
            "range": "± 3086",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_1000iters",
            "value": 214945,
            "range": "± 9073",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_10000iters",
            "value": 2340773,
            "range": "± 4506",
            "unit": "ns/iter"
          },
          {
            "name": "vm_scale/heavy_test/180_instrs_100000iters",
            "value": 23423116,
            "range": "± 115669",
            "unit": "ns/iter"
          },
          {
            "name": "runtime_feed/heavy_test_10k",
            "value": 2543491,
            "range": "± 6967",
            "unit": "ns/iter"
          }
        ]
      }
    ]
  }
}