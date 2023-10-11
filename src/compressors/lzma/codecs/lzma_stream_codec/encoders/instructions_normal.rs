use std::collections::VecDeque;

use crate::compressors::lzma::codecs::{
    length_codec::{MATCH_LEN_MAX, MATCH_LEN_MIN},
    lzma_stream_codec::{
        prices::{AnyRepPrice, NormalMatchPrice},
        state::State,
        EncoderPriceCalc, LZMACodec,
    },
    range_codec::RangeEncPrice,
};

use super::{
    match_finding::{Match, MatchFinder},
    EncodeInstruction, LZMAEncoderInput, LZMAInstructionPicker, LiteralCtx,
};

const MAX_NODE_GRAPH_LEN: usize = 4096 - MATCH_LEN_MAX;

pub struct LZMANormalInstructionPicker {
    nice_len: u32,
    pos_mask: u32,

    /// The "graph" of instructions and price nodes.
    /// The nodes are picked based on the current node and the length, and are assigned
    /// if they are cheaper than the destination node's current price. After that,
    /// the graph is traversed in reverse to find the best price path.
    /// This is somewhat similar to Dijkstra's path finding algorithm.
    node_graph: Vec<PriceNode>,
    graph_start_pos: u64,

    instruction_cache_stack: Vec<EncodeInstruction>,
}

impl LZMANormalInstructionPicker {
    const OPTS: u32 = 4096;

    pub fn new(nice_len: u32, pb: u32) -> Self {
        Self {
            nice_len,
            pos_mask: (1 << pb) - 1,

            node_graph: Vec::new(),
            graph_start_pos: 0,

            instruction_cache_stack: Vec::new(),
        }
    }

    fn ensure_capacity_for_pos(&mut self, pos: usize) {
        let capacity = pos + 1;
        if self.node_graph.len() < capacity {
            self.node_graph.resize(capacity, PriceNode::none());
        }
    }

    fn get_curr_node_index(&self, input: &LZMAEncoderInput<impl MatchFinder>) -> usize {
        let pos = input.pos();
        debug_assert!(pos >= self.graph_start_pos);
        (pos - self.graph_start_pos) as usize
    }

    fn reset_and_prepare_graph(
        &mut self,
        input: &LZMAEncoderInput<impl MatchFinder>,
        state: State,
    ) {
        self.node_graph.clear();
        self.node_graph.push(PriceNode::initial(state));
        self.graph_start_pos = input.pos() as u64;
    }

    /// Try:
    /// - Single literal
    /// - Single short rep
    /// - Literal + rep0
    fn try_one_length_opts(
        &mut self,
        input: &LZMAEncoderInput<impl MatchFinder>,
        price_calc: &EncoderPriceCalc,
        any_rep_price: AnyRepPrice,
    ) {
        let current_node_idx = self.get_curr_node_index(input);
        let node = self.node_graph[current_node_idx];
        let rep0 = node.state.get_rep(0);
        let price = node.price;

        let pos = input.pos();
        let available = input.buffer().forwards_bytes().min(MATCH_LEN_MAX); // We assume this is >= 1

        let curr_byte = input.buffer().get_byte(0);
        let prev_byte = input.buffer().get_byte(-1);
        let match_byte = input.buffer().get_byte(-(rep0 as i32) - 1);

        let literal_ctx = LiteralCtx {
            byte: curr_byte,
            match_byte,
            prev_byte,
        };

        // Literal price
        let price_literal = price
            + price_calc.get_literal_price(
                curr_byte,
                match_byte,
                prev_byte,
                pos as u32,
                &node.state,
            );

        let can_be_short_rep0 = curr_byte == match_byte;

        // Short rep price
        let price_short_rep = price + any_rep_price.get_short_rep_price();

        self.ensure_capacity_for_pos(current_node_idx + 1);

        let next_node = &mut self.node_graph[current_node_idx + 1];

        // Check if either of the options are cheaper than the next node's price.
        let min_price = price_literal.min(price_short_rep);
        if next_node.price > min_price {
            // Check which one was cheaper, and assign the node accordingly.
            if price_short_rep < price_literal && can_be_short_rep0 {
                *next_node = node.add_short_rep(price_short_rep);
            } else {
                *next_node = node.add_literal(price_literal, literal_ctx);
            }
        }

        // Get rep0 length
        let rep0_len = input.buffer().get_match_length(1, rep0, available as u32) - 1;

        // Get the rep0 price of the next position ahead
        let mut lit_state = node.state;
        lit_state.update_literal();
        let next_pos_state = (pos + 1) as u32 & self.pos_mask;
        let rep0_price = price_calc
            .get_any_match_price(&lit_state, next_pos_state)
            .get_any_rep_price()
            .get_long_rep_price(0);

        if rep0_len >= MATCH_LEN_MIN as u32 {
            let price_lit_rep0 = price_literal + rep0_price.get_price_with_len(rep0_len);

            let index = current_node_idx + rep0_len as usize + 1;
            self.ensure_capacity_for_pos(index);
            let next_node = &mut self.node_graph[index];

            if next_node.price > price_lit_rep0 {
                *next_node = node.add_lit_rep0(price_lit_rep0, literal_ctx, rep0_len);
            }
        }
    }

    /// Try:
    /// - All reps
    fn try_reps(&mut self, input: &LZMAEncoderInput<impl MatchFinder>, any_rep_price: AnyRepPrice) {
        let current_node_idx = self.get_curr_node_index(input);
        let node = self.node_graph[current_node_idx];
        let price = node.price;

        let available = input.buffer().forwards_bytes().min(MATCH_LEN_MAX);

        for rep_id in 0..node.state.reps().len() {
            let rep_dist = node.state.get_rep(rep_id);
            let rep_len = input
                .buffer()
                .get_match_length(0, rep_dist, available as u32);

            if rep_len < MATCH_LEN_MIN as u32 {
                continue;
            }

            self.ensure_capacity_for_pos(current_node_idx + rep_len as usize);

            let rep_price = any_rep_price.get_long_rep_price(rep_id as u32);

            // Try the prices of all lengths of this rep
            for rep_len in (MATCH_LEN_MIN as u32)..=rep_len {
                let price_rep = price + rep_price.get_price_with_len(rep_len);

                let index = current_node_idx + rep_len as usize;
                let next_node = &mut self.node_graph[index];

                if next_node.price > price_rep {
                    *next_node = node.add_long_rep(price_rep, rep_id, rep_len);
                }
            }
        }
    }

    /// Try:
    /// - All matches
    fn try_matches(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        normal_match_price: NormalMatchPrice,
    ) {
        let current_node_idx = self.get_curr_node_index(input);
        let node = self.node_graph[current_node_idx];
        let price = node.price;

        let matches = input.calc_matches();

        for match_ in matches {
            if node.state.reps().contains(&match_.distance) {
                // If we've already checked this as a rep, skip it.
                continue;
            }

            self.ensure_capacity_for_pos(current_node_idx + match_.len as usize);

            for match_len in (MATCH_LEN_MIN as u32)..=match_.len {
                let price_rep =
                    price + normal_match_price.get_price_with_dist_len(match_.distance, match_len);

                let index = current_node_idx + match_len as usize;
                let next_node = &mut self.node_graph[index];

                if next_node.price > price_rep {
                    *next_node = node.add_match(price_rep, match_len, match_.distance);
                }
            }
        }
    }

    fn convert_graph_into_instructions(&mut self) {
        let mut pos = self.node_graph.len() - 1;

        while pos != 0 {
            let node = self.node_graph[pos];

            match node.instruction {
                NodeInstruction::None => {
                    unreachable!();
                }
                NodeInstruction::Literal { ctx } => {
                    let instruction = EncodeInstruction::Literal(ctx);
                    self.instruction_cache_stack.push(instruction);
                }
                NodeInstruction::Rep { rep_index } => {
                    let instruction = EncodeInstruction::Rep {
                        rep_index,
                        len: node.len,
                    };
                    self.instruction_cache_stack.push(instruction);
                }
                NodeInstruction::Match { distance } => {
                    let instruction = EncodeInstruction::Match(Match {
                        distance,
                        len: node.len,
                    });
                    self.instruction_cache_stack.push(instruction);
                }
                NodeInstruction::LiteralThenRep0 { literal_ctx } => {
                    // Add them in reverse as it's a stack
                    let instruction = EncodeInstruction::Rep {
                        rep_index: 0,
                        len: node.len - 1,
                    };
                    self.instruction_cache_stack.push(instruction);

                    let instruction = EncodeInstruction::Literal(literal_ctx);
                    self.instruction_cache_stack.push(instruction);
                }
            }

            pos -= node.len as usize;
        }
    }
}

impl LZMAInstructionPicker for LZMANormalInstructionPicker {
    fn get_next_symbol(
        &mut self,
        input: &mut LZMAEncoderInput<impl MatchFinder>,
        price_calc: &mut EncoderPriceCalc,
        state: &State,
    ) -> EncodeInstruction {
        if let Some(instruction) = self.instruction_cache_stack.pop() {
            return instruction;
        }

        let avail = usize::min(input.forward_bytes(), MATCH_LEN_MAX) as u32;

        if avail < MATCH_LEN_MIN as u32 {
            // Just return a literal
            let next_byte = input.buffer().get_byte(0);
            let prev_byte = input.buffer().get_byte(-1);
            let match_byte = input.buffer().get_byte(-(state.reps()[0] as i32) - 1);
            let literal_ctx = LiteralCtx {
                byte: next_byte,
                match_byte,
                prev_byte,
            };

            return EncodeInstruction::Literal(literal_ctx);
        }

        self.reset_and_prepare_graph(input, *state);

        while self.node_graph.len() < MAX_NODE_GRAPH_LEN
            && self.get_curr_node_index(input) < self.node_graph.len()
            && input.forward_bytes() > 0
        {
            let pos = input.pos();
            let pos_state = pos as u32 & self.pos_mask;

            let any_match_price = price_calc.get_any_match_price(state, pos_state);
            let any_rep_price = any_match_price.get_any_rep_price();
            let normal_match_price = any_match_price.get_normal_match_price();

            self.try_one_length_opts(input, price_calc, any_rep_price);
            self.try_reps(input, any_rep_price);
            self.try_matches(input, normal_match_price);

            input.increment_pos();
        }

        self.convert_graph_into_instructions();

        return self.instruction_cache_stack.pop().unwrap();
    }
}

#[derive(Debug, Clone, Copy)]
enum NodeInstruction {
    None,
    Match { distance: u32 },
    Rep { rep_index: usize },
    Literal { ctx: LiteralCtx },

    // This is a very common case so we include it as its own instruction.
    // The rep length is len - 1.
    LiteralThenRep0 { literal_ctx: LiteralCtx },
}

#[derive(Debug, Clone, Copy)]
struct PriceNode {
    instruction: NodeInstruction,
    state: State,
    len: u32,

    price: RangeEncPrice,
}

impl PriceNode {
    pub fn none() -> Self {
        Self {
            instruction: NodeInstruction::None,
            state: State::new(),
            len: 0,
            price: RangeEncPrice::infinity(),
        }
    }

    pub fn initial(state: State) -> Self {
        Self {
            instruction: NodeInstruction::None, // Some default value, won't be used
            state,
            len: 0,
            price: RangeEncPrice::zero(),
        }
    }

    #[inline(always)]
    pub fn add_literal(&self, price: RangeEncPrice, ctx: LiteralCtx) -> Self {
        let mut new_state = self.state;
        new_state.update_literal();
        Self {
            instruction: NodeInstruction::Literal { ctx },
            state: new_state,
            len: 1,
            price,
        }
    }

    #[inline(always)]
    pub fn add_short_rep(&self, price: RangeEncPrice) -> Self {
        let mut new_state = self.state;
        new_state.update_short_rep();
        Self {
            instruction: NodeInstruction::Rep { rep_index: 0 },
            state: new_state,
            len: 1,
            price,
        }
    }

    #[inline(always)]
    pub fn add_long_rep(&self, price: RangeEncPrice, rep: usize, len: u32) -> Self {
        let mut new_state = self.state;
        new_state.update_long_rep(rep);
        Self {
            instruction: NodeInstruction::Rep { rep_index: rep },
            state: new_state,
            len,
            price,
        }
    }

    #[inline(always)]
    pub fn add_match(&self, price: RangeEncPrice, len: u32, distance: u32) -> Self {
        let mut new_state = self.state;
        new_state.update_match(distance);
        Self {
            instruction: NodeInstruction::Match { distance },
            state: new_state,
            len,
            price,
        }
    }

    pub fn add_lit_rep0(
        &self,
        price: RangeEncPrice,
        literal_ctx: LiteralCtx,
        long_rep_len: u32,
    ) -> Self {
        let mut new_state = self.state;
        new_state.update_literal();
        new_state.update_long_rep(0);
        Self {
            instruction: NodeInstruction::LiteralThenRep0 { literal_ctx },
            state: new_state,
            len: long_rep_len + 1,
            price,
        }
    }
}
