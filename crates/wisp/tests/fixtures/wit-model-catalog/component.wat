(module
  (memory (export "memory") 1)
  (global $heap (mut i32) (i32.const 1024))

  (func (export "cabi_realloc")
    (param $old-pointer i32) (param $old-size i32) (param $alignment i32) (param $new-size i32)
    (result i32)
    (local $pointer i32)
    (local.set $pointer
      (i32.and
        (i32.add (global.get $heap) (i32.sub (local.get $alignment) (i32.const 1)))
        (i32.sub (i32.const 0) (local.get $alignment))))
    (global.set $heap (i32.add (local.get $pointer) (local.get $new-size)))
    (local.get $pointer))

  (data (i32.const 64) "sku required")
  (data (i32.const 80) "books")
  (data (i32.const 88) "games")
  (data (i32.const 96) "tools")
  (data (i32.const 104) "zero")
  (data (i32.const 136) "normal")
  (data (i32.const 144) "max")
  (data (i32.const 160) "available")
  (data (i32.const 176) "unavailable")
  (data (i32.const 220) "negative")
  (data (i32.const 228) "positive")
  (data (i32.const 236) "min")
  (data (i32.const 240) "max")
  (data (i32.const 500) "ascii")
  (data (i32.const 505) "bmp")
  (data (i32.const 508) "astral")
  (data (i32.const 520) "negative-zero")
  (data (i32.const 534) "fractional")
  (data (i32.const 545) "nan")
  (data (i32.const 548) "positive-infinity")
  (data (i32.const 565) "negative-infinity")
  (data (i32.const 680) "representative")
  (data (i32.const 700) "beyond-safe")
  (data (i32.const 720) "negative-beyond-safe")
  (data (i32.const 840) "standard")
  (data (i32.const 848) "next-day")
  (data (i32.const 856) "same-day")
  (data (i32.const 900) "read-only")
  (data (i32.const 909) "write")
  (data (i32.const 914) "execute")
  (data (i32.const 990) "all")
  (data (i32.const 984) "none")
  (data (i32.const 1000) "ok")
  (data (i32.const 1004) "err")
  (data (i32.const 1010) "empty")
  (data (i32.const 1015) "nonempty")

  (func (export "lookup")
    (param $sku-pointer i32) (param $sku-size i32)
    (param $locale-pointer i32) (param $locale-size i32)
    (result i32)
    (if (i32.eqz (local.get $sku-size))
      (then
        (i32.store (i32.const 0) (i32.const 1))
        (i32.store (i32.const 4) (i32.const 64))
        (i32.store (i32.const 8) (i32.const 12)))
      (else
        (i32.store (i32.const 0) (i32.const 0))
        (i32.store (i32.const 4) (i32.const 32))
        (i32.store (i32.const 8) (i32.const 2))
        (i32.store (i32.const 32) (local.get $sku-pointer))
        (i32.store (i32.const 36) (local.get $sku-size))
        (i32.store (i32.const 40) (local.get $locale-pointer))
        (i32.store (i32.const 44) (local.get $locale-size))))
    (i32.const 0))

  (func (export "cabi_post_lookup") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "list-categories") (result i32)
    (i32.store (i32.const 16) (i32.const 0))
    (i32.store (i32.const 20) (i32.const 112))
    (i32.store (i32.const 24) (i32.const 3))
    (i32.store (i32.const 112) (i32.const 80))
    (i32.store (i32.const 116) (i32.const 5))
    (i32.store (i32.const 120) (i32.const 88))
    (i32.store (i32.const 124) (i32.const 5))
    (i32.store (i32.const 128) (i32.const 96))
    (i32.store (i32.const 132) (i32.const 5))
    (i32.const 16))

  (func (export "cabi_post_list-categories") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "stock-labels") (param $count i32) (result i32)
    (i32.store (i32.const 48) (i32.const 0))
    (i32.store (i32.const 52) (i32.const 152))
    (i32.store (i32.const 56) (i32.const 1))
    (if (i32.eqz (local.get $count))
      (then
        (i32.store (i32.const 152) (i32.const 104))
        (i32.store (i32.const 156) (i32.const 4)))
      (else
        (if (i32.eq (local.get $count) (i32.const -1))
          (then
            (i32.store (i32.const 152) (i32.const 144))
            (i32.store (i32.const 156) (i32.const 3)))
          (else
            (i32.store (i32.const 152) (i32.const 136))
            (i32.store (i32.const 156) (i32.const 6))))))
    (i32.const 48))

  (func (export "cabi_post_stock-labels") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "availability-label") (param $available i32) (result i32)
    (i32.store (i32.const 192) (i32.const 0))
    (i32.store (i32.const 196) (i32.const 204))
    (i32.store (i32.const 200) (i32.const 1))
    (if (local.get $available)
      (then
        (i32.store (i32.const 204) (i32.const 160))
        (i32.store (i32.const 208) (i32.const 9)))
      (else
        (i32.store (i32.const 204) (i32.const 176))
        (i32.store (i32.const 208) (i32.const 11))))
    (i32.const 192))

  (func (export "cabi_post_availability-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "adjustment-label") (param $value i32) (result i32)
    (i32.store (i32.const 256) (i32.const 0))
    (i32.store (i32.const 260) (i32.const 268))
    (i32.store (i32.const 264) (i32.const 1))
    (if (i32.eq (local.get $value) (i32.const -2147483648))
      (then
        (i32.store (i32.const 268) (i32.const 236))
        (i32.store (i32.const 272) (i32.const 3)))
      (else
        (if (i32.eq (local.get $value) (i32.const 2147483647))
          (then
            (i32.store (i32.const 268) (i32.const 240))
            (i32.store (i32.const 272) (i32.const 3)))
          (else
            (if (i32.eqz (local.get $value))
              (then
                (i32.store (i32.const 268) (i32.const 104))
                (i32.store (i32.const 272) (i32.const 4)))
              (else
                (if (i32.lt_s (local.get $value) (i32.const 0))
                  (then
                    (i32.store (i32.const 268) (i32.const 220))
                    (i32.store (i32.const 272) (i32.const 8)))
                  (else
                    (i32.store (i32.const 268) (i32.const 228))
                    (i32.store (i32.const 272) (i32.const 8))))))))))
    (i32.const 256))

  (func (export "cabi_post_adjustment-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "priority-label") (param $value i32) (result i32)
    (i32.store (i32.const 288) (i32.const 0))
    (i32.store (i32.const 292) (i32.const 300))
    (i32.store (i32.const 296) (i32.const 1))
    (if (i32.eqz (local.get $value))
      (then
        (i32.store (i32.const 300) (i32.const 104))
        (i32.store (i32.const 304) (i32.const 4)))
      (else
        (if (i32.eq (local.get $value) (i32.const 255))
          (then
            (i32.store (i32.const 300) (i32.const 144))
            (i32.store (i32.const 304) (i32.const 3)))
          (else
            (i32.store (i32.const 300) (i32.const 136))
            (i32.store (i32.const 304) (i32.const 6))))))
    (i32.const 288))

  (func (export "cabi_post_priority-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "rank-label") (param $value i32) (result i32)
    (i32.store (i32.const 320) (i32.const 0))
    (i32.store (i32.const 324) (i32.const 332))
    (i32.store (i32.const 328) (i32.const 1))
    (if (i32.eqz (local.get $value))
      (then
        (i32.store (i32.const 332) (i32.const 104))
        (i32.store (i32.const 336) (i32.const 4)))
      (else
        (if (i32.eq (local.get $value) (i32.const 65535))
          (then
            (i32.store (i32.const 332) (i32.const 144))
            (i32.store (i32.const 336) (i32.const 3)))
          (else
            (i32.store (i32.const 332) (i32.const 136))
            (i32.store (i32.const 336) (i32.const 6))))))
    (i32.const 320))

  (func (export "cabi_post_rank-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "temperature-label") (param $value i32) (result i32)
    (i32.store (i32.const 352) (i32.const 0))
    (i32.store (i32.const 356) (i32.const 364))
    (i32.store (i32.const 360) (i32.const 1))
    (if (i32.eq (local.get $value) (i32.const -128))
      (then
        (i32.store (i32.const 364) (i32.const 236))
        (i32.store (i32.const 368) (i32.const 3)))
      (else
        (if (i32.eq (local.get $value) (i32.const 127))
          (then
            (i32.store (i32.const 364) (i32.const 240))
            (i32.store (i32.const 368) (i32.const 3)))
          (else
            (if (i32.eqz (local.get $value))
              (then
                (i32.store (i32.const 364) (i32.const 104))
                (i32.store (i32.const 368) (i32.const 4)))
              (else
                (if (i32.lt_s (local.get $value) (i32.const 0))
                  (then
                    (i32.store (i32.const 364) (i32.const 220))
                    (i32.store (i32.const 368) (i32.const 8)))
                  (else
                    (i32.store (i32.const 364) (i32.const 228))
                    (i32.store (i32.const 368) (i32.const 8))))))))))
    (i32.const 352))

  (func (export "cabi_post_temperature-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "pressure-label") (param $value i32) (result i32)
    (i32.store (i32.const 384) (i32.const 0))
    (i32.store (i32.const 388) (i32.const 396))
    (i32.store (i32.const 392) (i32.const 1))
    (if (i32.eq (local.get $value) (i32.const -32768))
      (then
        (i32.store (i32.const 396) (i32.const 236))
        (i32.store (i32.const 400) (i32.const 3)))
      (else
        (if (i32.eq (local.get $value) (i32.const 32767))
          (then
            (i32.store (i32.const 396) (i32.const 240))
            (i32.store (i32.const 400) (i32.const 3)))
          (else
            (if (i32.eqz (local.get $value))
              (then
                (i32.store (i32.const 396) (i32.const 104))
                (i32.store (i32.const 400) (i32.const 4)))
              (else
                (if (i32.lt_s (local.get $value) (i32.const 0))
                  (then
                    (i32.store (i32.const 396) (i32.const 220))
                    (i32.store (i32.const 400) (i32.const 8)))
                  (else
                    (i32.store (i32.const 396) (i32.const 228))
                    (i32.store (i32.const 400) (i32.const 8))))))))))
    (i32.const 384))

  (func (export "cabi_post_pressure-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "character-label") (param $value i32) (result i32)
    (i32.store (i32.const 416) (i32.const 0))
    (i32.store (i32.const 420) (i32.const 428))
    (i32.store (i32.const 424) (i32.const 1))
    (if (i32.eq (local.get $value) (i32.const 65))
      (then
        (i32.store (i32.const 428) (i32.const 500))
        (i32.store (i32.const 432) (i32.const 5)))
      (else
        (if (i32.eq (local.get $value) (i32.const 955))
          (then
            (i32.store (i32.const 428) (i32.const 505))
            (i32.store (i32.const 432) (i32.const 3)))
          (else
            (i32.store (i32.const 428) (i32.const 508))
            (i32.store (i32.const 432) (i32.const 6))))))
    (i32.const 416))

  (func (export "cabi_post_character-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "metric-label") (param $value f32) (result i32)
    (local $bits i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $bits (i32.reinterpret_f32 (local.get $value)))
    (local.set $pointer (i32.const 228))
    (local.set $size (i32.const 8))
    (if (i32.lt_s (local.get $bits) (i32.const 0))
      (then
        (local.set $pointer (i32.const 220))
        (local.set $size (i32.const 8))))
    (if (i32.eqz (i32.and (local.get $bits) (i32.const 2147483647)))
      (then
        (local.set $pointer (i32.const 104))
        (local.set $size (i32.const 4))))
    (if (i32.eq (local.get $bits) (i32.const -2147483648))
      (then
        (local.set $pointer (i32.const 520))
        (local.set $size (i32.const 13))))
    (if (i32.eq (local.get $bits) (i32.const 1069547520))
      (then
        (local.set $pointer (i32.const 534))
        (local.set $size (i32.const 10))))
    (if (i32.eq (local.get $bits) (i32.const 2139095040))
      (then
        (local.set $pointer (i32.const 548))
        (local.set $size (i32.const 17))))
    (if (i32.eq (local.get $bits) (i32.const -8388608))
      (then
        (local.set $pointer (i32.const 565))
        (local.set $size (i32.const 17))))
    (if (i32.gt_u
          (i32.and (local.get $bits) (i32.const 2147483647))
          (i32.const 2139095040))
      (then
        (local.set $pointer (i32.const 545))
        (local.set $size (i32.const 3))))
    (i32.store (i32.const 588) (i32.const 0))
    (i32.store (i32.const 592) (i32.const 600))
    (i32.store (i32.const 596) (i32.const 1))
    (i32.store (i32.const 600) (local.get $pointer))
    (i32.store (i32.const 604) (local.get $size))
    (i32.const 588))

  (func (export "cabi_post_metric-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "measurement-label") (param $value f64) (result i32)
    (local $bits i64)
    (local $pointer i32)
    (local $size i32)
    (local.set $bits (i64.reinterpret_f64 (local.get $value)))
    (local.set $pointer (i32.const 228))
    (local.set $size (i32.const 8))
    (if (i64.lt_s (local.get $bits) (i64.const 0))
      (then
        (local.set $pointer (i32.const 220))
        (local.set $size (i32.const 8))))
    (if (i64.eqz (i64.and (local.get $bits) (i64.const 9223372036854775807)))
      (then
        (local.set $pointer (i32.const 104))
        (local.set $size (i32.const 4))))
    (if (i64.eq (local.get $bits) (i64.const -9223372036854775808))
      (then
        (local.set $pointer (i32.const 520))
        (local.set $size (i32.const 13))))
    (if (i64.eq (local.get $bits) (i64.const 4609434218613702656))
      (then
        (local.set $pointer (i32.const 534))
        (local.set $size (i32.const 10))))
    (if (i64.eq (local.get $bits) (i64.const 9218868437227405312))
      (then
        (local.set $pointer (i32.const 548))
        (local.set $size (i32.const 17))))
    (if (i64.eq (local.get $bits) (i64.const -4503599627370496))
      (then
        (local.set $pointer (i32.const 565))
        (local.set $size (i32.const 17))))
    (if (i64.gt_u
          (i64.and (local.get $bits) (i64.const 9223372036854775807))
          (i64.const 9218868437227405312))
      (then
        (local.set $pointer (i32.const 545))
        (local.set $size (i32.const 3))))
    (i32.store (i32.const 620) (i32.const 0))
    (i32.store (i32.const 624) (i32.const 632))
    (i32.store (i32.const 628) (i32.const 1))
    (i32.store (i32.const 632) (local.get $pointer))
    (i32.store (i32.const 636) (local.get $size))
    (i32.const 620))

  (func (export "cabi_post_measurement-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "serial-label") (param $value i64) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 136))
    (local.set $size (i32.const 6))
    (if (i64.eqz (local.get $value))
      (then
        (local.set $pointer (i32.const 104))
        (local.set $size (i32.const 4))))
    (if (i64.eq (local.get $value) (i64.const 42))
      (then
        (local.set $pointer (i32.const 680))
        (local.set $size (i32.const 14))))
    (if (i64.eq (local.get $value) (i64.const 9007199254740993))
      (then
        (local.set $pointer (i32.const 700))
        (local.set $size (i32.const 11))))
    (if (i64.eq (local.get $value) (i64.const -1))
      (then
        (local.set $pointer (i32.const 144))
        (local.set $size (i32.const 3))))
    (i32.store (i32.const 648) (i32.const 0))
    (i32.store (i32.const 652) (i32.const 660))
    (i32.store (i32.const 656) (i32.const 1))
    (i32.store (i32.const 660) (local.get $pointer))
    (i32.store (i32.const 664) (local.get $size))
    (i32.const 648))

  (func (export "cabi_post_serial-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "signed-serial-label") (param $value i64) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 680))
    (local.set $size (i32.const 14))
    (if (i64.eq (local.get $value) (i64.const -9223372036854775808))
      (then
        (local.set $pointer (i32.const 236))
        (local.set $size (i32.const 3))))
    (if (i64.eq (local.get $value) (i64.const -9007199254740993))
      (then
        (local.set $pointer (i32.const 720))
        (local.set $size (i32.const 20))))
    (if (i64.eq (local.get $value) (i64.const -1))
      (then
        (local.set $pointer (i32.const 220))
        (local.set $size (i32.const 8))))
    (if (i64.eqz (local.get $value))
      (then
        (local.set $pointer (i32.const 104))
        (local.set $size (i32.const 4))))
    (if (i64.eq (local.get $value) (i64.const 9007199254740993))
      (then
        (local.set $pointer (i32.const 700))
        (local.set $size (i32.const 11))))
    (if (i64.eq (local.get $value) (i64.const 9223372036854775807))
      (then
        (local.set $pointer (i32.const 144))
        (local.set $size (i32.const 3))))
    (i32.store (i32.const 760) (i32.const 0))
    (i32.store (i32.const 764) (i32.const 772))
    (i32.store (i32.const 768) (i32.const 1))
    (i32.store (i32.const 772) (local.get $pointer))
    (i32.store (i32.const 776) (local.get $size))
    (i32.const 760))

  (func (export "cabi_post_signed-serial-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "search")
    (param $term-pointer i32) (param $term-size i32)
    (param $limit i32) (param $include-disabled i32)
    (result i32)
    (i32.store (i32.const 800) (i32.const 0))
    (i32.store (i32.const 804) (i32.const 812))
    (i32.store (i32.const 808) (i32.const 3))
    (i32.store (i32.const 812) (local.get $term-pointer))
    (i32.store (i32.const 816) (local.get $term-size))
    (if (i32.eqz (local.get $limit))
      (then
        (i32.store (i32.const 820) (i32.const 104))
        (i32.store (i32.const 824) (i32.const 4)))
      (else
        (if (i32.eq (local.get $limit) (i32.const -1))
          (then
            (i32.store (i32.const 820) (i32.const 144))
            (i32.store (i32.const 824) (i32.const 3)))
          (else
            (i32.store (i32.const 820) (i32.const 136))
            (i32.store (i32.const 824) (i32.const 6))))))
    (if (local.get $include-disabled)
      (then
        (i32.store (i32.const 828) (i32.const 160))
        (i32.store (i32.const 832) (i32.const 9)))
      (else
        (i32.store (i32.const 828) (i32.const 176))
        (i32.store (i32.const 832) (i32.const 11))))
    (i32.const 800))

  (func (export "cabi_post_search") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "delivery-label") (param $speed i32) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 840))
    (local.set $size (i32.const 8))
    (if (i32.eq (local.get $speed) (i32.const 1))
      (then
        (local.set $pointer (i32.const 848))
        (local.set $size (i32.const 8))))
    (if (i32.eq (local.get $speed) (i32.const 2))
      (then
        (local.set $pointer (i32.const 856))
        (local.set $size (i32.const 8))))
    (i32.store (i32.const 880) (i32.const 0))
    (i32.store (i32.const 884) (i32.const 892))
    (i32.store (i32.const 888) (i32.const 1))
    (i32.store (i32.const 892) (local.get $pointer))
    (i32.store (i32.const 896) (local.get $size))
    (i32.const 880))

  (func (export "cabi_post_delivery-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "access-label") (param $modes i32) (result i32)
    (local $count i32)
    (i32.store (i32.const 928) (i32.const 0))
    (i32.store (i32.const 932) (i32.const 944))
    (if (i32.and (local.get $modes) (i32.const 1))
      (then
        (i32.store (i32.const 944) (i32.const 900))
        (i32.store (i32.const 948) (i32.const 9))
        (local.set $count (i32.const 1))))
    (if (i32.and (local.get $modes) (i32.const 2))
      (then
        (i32.store
          (i32.add (i32.const 944) (i32.mul (local.get $count) (i32.const 8)))
          (i32.const 909))
        (i32.store
          (i32.add (i32.const 948) (i32.mul (local.get $count) (i32.const 8)))
          (i32.const 5))
        (local.set $count (i32.add (local.get $count) (i32.const 1)))))
    (if (i32.and (local.get $modes) (i32.const 4))
      (then
        (i32.store
          (i32.add (i32.const 944) (i32.mul (local.get $count) (i32.const 8)))
          (i32.const 914))
        (i32.store
          (i32.add (i32.const 948) (i32.mul (local.get $count) (i32.const 8)))
          (i32.const 7))
        (local.set $count (i32.add (local.get $count) (i32.const 1)))))
    (i32.store (i32.const 936) (local.get $count))
    (i32.const 928))

  (func (export "cabi_post_access-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "lookup-by")
    (param $case i32) (param $payload0 i32) (param $payload1 i32) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 990))
    (local.set $size (i32.const 3))
    (if (i32.eq (local.get $case) (i32.const 1))
      (then
        (local.set $pointer (local.get $payload0))
        (local.set $size (local.get $payload1))))
    (if (i32.eq (local.get $case) (i32.const 2))
      (then
        (local.set $pointer (i32.const 136))
        (local.set $size (i32.const 6))
        (if (i32.eqz (local.get $payload0))
          (then (local.set $pointer (i32.const 104)) (local.set $size (i32.const 4))))
        (if (i32.eq (local.get $payload0) (i32.const -1))
          (then (local.set $pointer (i32.const 144)) (local.set $size (i32.const 3))))))
    (if (i32.eq (local.get $case) (i32.const 3))
      (then
        (if (local.get $payload0)
          (then (local.set $pointer (i32.const 160)) (local.set $size (i32.const 9)))
          (else (local.set $pointer (i32.const 176)) (local.set $size (i32.const 11))))))
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (i32.store (i32.const 972) (local.get $pointer))
    (i32.store (i32.const 976) (local.get $size))
    (i32.const 960))

  (func (export "cabi_post_lookup-by") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "optional-sku-label")
    (param $some i32) (param $pointer i32) (param $size i32) (result i32)
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (if (local.get $some)
      (then
        (i32.store (i32.const 972) (local.get $pointer))
        (i32.store (i32.const 976) (local.get $size)))
      (else
        (i32.store (i32.const 972) (i32.const 984))
        (i32.store (i32.const 976) (i32.const 4))))
    (i32.const 960))

  (func (export "cabi_post_optional-sku-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "optional-quantity-label")
    (param $some i32) (param $value i32) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 984))
    (local.set $size (i32.const 4))
    (if (local.get $some)
      (then
        (local.set $pointer (i32.const 136))
        (local.set $size (i32.const 6))
        (if (i32.eqz (local.get $value))
          (then
            (local.set $pointer (i32.const 104))
            (local.set $size (i32.const 4))))))
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (i32.store (i32.const 972) (local.get $pointer))
    (i32.store (i32.const 976) (local.get $size))
    (i32.const 960))

  (func (export "cabi_post_optional-quantity-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "result-label")
    (param $case i32) (param $payload0 i32) (param $payload1 i32) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (local.get $payload0))
    (local.set $size (local.get $payload1))
    (if (local.get $case)
      (then
        (local.set $pointer (i32.const 136))
        (local.set $size (i32.const 6))
        (if (i32.eqz (local.get $payload0))
          (then (local.set $pointer (i32.const 104)) (local.set $size (i32.const 4))))
        (if (i32.eq (local.get $payload0) (i32.const -1))
          (then (local.set $pointer (i32.const 144)) (local.set $size (i32.const 3))))))
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (i32.store (i32.const 972) (local.get $pointer))
    (i32.store (i32.const 976) (local.get $size))
    (i32.const 960))

  (func (export "cabi_post_result-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "status-label")
    (param $case i32) (param $pointer i32) (param $size i32) (result i32)
    (if (i32.eqz (local.get $case))
      (then
        (i32.store (i32.const 972) (i32.const 1000))
        (i32.store (i32.const 976) (i32.const 2)))
      (else
        (i32.store (i32.const 972) (local.get $pointer))
        (i32.store (i32.const 976) (local.get $size))))
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (i32.const 960))

  (func (export "cabi_post_status-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func (export "quantity-result-label")
    (param $case i32) (param $value i32) (result i32)
    (local $pointer i32)
    (local $size i32)
    (local.set $pointer (i32.const 1004))
    (local.set $size (i32.const 3))
    (if (i32.eqz (local.get $case))
      (then
        (local.set $pointer (i32.const 136))
        (local.set $size (i32.const 6))
        (if (i32.eqz (local.get $value))
          (then (local.set $pointer (i32.const 104)) (local.set $size (i32.const 4))))))
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (i32.store (i32.const 972) (local.get $pointer))
    (i32.store (i32.const 976) (local.get $size))
    (i32.const 960))

  (func (export "cabi_post_quantity-result-label") (param i32)
    (global.set $heap (i32.const 1024)))

  (func $accept-list (param $pointer i32) (param $length i32) (result i32)
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (if (i32.eqz (local.get $length))
      (then
        (i32.store (i32.const 972) (i32.const 1010))
        (i32.store (i32.const 976) (i32.const 5)))
      (else
        (i32.store (i32.const 972) (i32.const 1015))
        (i32.store (i32.const 976) (i32.const 8))))
    (i32.const 960))
  (export "accept-strings" (func $accept-list))
  (export "accept-u8s" (func $accept-list))
  (export "accept-u16s" (func $accept-list))
  (export "accept-u32s" (func $accept-list))
  (export "accept-u64s" (func $accept-list))
  (export "accept-s8s" (func $accept-list))
  (export "accept-s16s" (func $accept-list))
  (export "accept-s32s" (func $accept-list))
  (export "accept-s64s" (func $accept-list))
  (export "accept-f32s" (func $accept-list))
  (export "accept-f64s" (func $accept-list))
  (export "accept-bools" (func $accept-list))
  (export "accept-chars" (func $accept-list))

  (func $post-list (param i32) (global.set $heap (i32.const 1024)))
  (export "cabi_post_accept-strings" (func $post-list))
  (export "cabi_post_accept-u8s" (func $post-list))
  (export "cabi_post_accept-u16s" (func $post-list))
  (export "cabi_post_accept-u32s" (func $post-list))
  (export "cabi_post_accept-u64s" (func $post-list))
  (export "cabi_post_accept-s8s" (func $post-list))
  (export "cabi_post_accept-s16s" (func $post-list))
  (export "cabi_post_accept-s32s" (func $post-list))
  (export "cabi_post_accept-s64s" (func $post-list))
  (export "cabi_post_accept-f32s" (func $post-list))
  (export "cabi_post_accept-f64s" (func $post-list))
  (export "cabi_post_accept-bools" (func $post-list))
  (export "cabi_post_accept-chars" (func $post-list))

  (func (export "accept-product-key")
    (param $text-pointer i32) (param $text-size i32) (param $quantity i32)
    (param $enabled i32) (param $symbol i32) (param $serial i64) (result i32)
    (i32.store (i32.const 960) (i32.const 0))
    (i32.store (i32.const 964) (i32.const 972))
    (i32.store (i32.const 968) (i32.const 1))
    (if
      (i32.and
        (i32.and
          (i32.eq (local.get $text-size) (i32.const 6))
          (i32.eq (i32.load (local.get $text-pointer)) (i32.const 760564563)))
        (i32.and
          (i32.and
            (i32.eq (i32.load16_u offset=4 (local.get $text-pointer)) (i32.const 12852))
            (i32.eq (local.get $quantity) (i32.const -1)))
          (i32.and
            (i32.eq (local.get $enabled) (i32.const 1))
            (i32.and
              (i32.eq (local.get $symbol) (i32.const 128512))
              (i64.eq (local.get $serial) (i64.const -1))))))
      (then
        (i32.store (i32.const 972) (i32.const 144))
        (i32.store (i32.const 976) (i32.const 3)))
      (else
        (i32.store (i32.const 972) (i32.const 680))
        (i32.store (i32.const 976) (i32.const 14))))
    (i32.const 960))
  (export "cabi_post_accept-product-key" (func $post-list))
)
