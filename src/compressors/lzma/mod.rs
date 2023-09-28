pub mod length_codec;
pub mod range_codec;

// private static readonly byte[] G_FAST_POS = new byte[1 << 11];

// static Encoder()
// {
//     const byte kFastSlots = 22;
//     var c = 2;
//     G_FAST_POS[0] = 0;
//     G_FAST_POS[1] = 1;
//     for (byte slotFast = 2; slotFast < kFastSlots; slotFast++)
//     {
//         var k = ((uint)1 << ((slotFast >> 1) - 1));
//         for (uint j = 0; j < k; j++, c++)
//         {
//             G_FAST_POS[c] = slotFast;
//         }
//     }
// }

use lazy_static::lazy_static;

lazy_static! {
    pub static ref G_FAST_POS: [u8; 1 << 11] = {
        let mut g_fast_pos = [0; 1 << 11];
        let k_fast_slots = 22;
        let mut c = 2;
        g_fast_pos[0] = 0;
        g_fast_pos[1] = 1;
        for slot_fast in 2..k_fast_slots {
            let k = 1 << ((slot_fast >> 1) - 1);
            for _ in 0..k {
                g_fast_pos[c] = slot_fast;
                c += 1;
            }
        }
        g_fast_pos
    };
}
