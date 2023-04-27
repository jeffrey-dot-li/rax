use dfdx::{
    data::Arange,
    nn::modules::*,
    shapes::*,
    tensor::{DeviceStorage, Tensor},
    tensor_ops::*,
};

use super::lazy::LazyTensor;

const VOCAB: usize = 32_000;
const HIDDEN: usize = 4096;
const INTERMEDIATE: usize = 11008;
const NUM_HEADS: usize = 32;
const NUM_LAYERS: usize = 32;
const HEAD_DIM: usize = HIDDEN / NUM_HEADS;
const HEAD_DIM_OVER_2: usize = HEAD_DIM / 2;

type E = half::f16;

struct RMSNorm {
    weight: LazyTensor<(Const<HIDDEN>,), E>,
    variance_epsilon: f64,
}

impl RMSNorm {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E>>(
        &self,
        x: Tensor<(Batch, Seq, Const<HIDDEN>), E, D>,
    ) -> Tensor<(Batch, Seq, Const<HIDDEN>), E, D> {
        let weight = self.weight.load_on(x.device());
        let (batch, seq, _) = *x.shape();
        let variance = x.clone().square().mean::<(Batch, Seq), _>();
        let inv_std = (variance + self.variance_epsilon as f32).sqrt().recip();
        let x = x * inv_std.broadcast_like(&(batch, seq, Const));
        x * weight.broadcast_like(&(batch, seq, Const))
    }
}

struct RotaryEmbedding {
    inv_freq: LazyTensor<Rank1<HEAD_DIM_OVER_2>, f32>,
}

impl RotaryEmbedding {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E> + Device<f32>>(
        &self,
        q: Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D>,
        k: Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D>,
    ) -> (
        Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D>,
        Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D>,
    ) {
        let (sin, cos) = self.get_sincos(q.device(), q.shape().2);
        let cos = cos.broadcast_like(q.shape());
        let sin = sin.broadcast_like(q.shape());
        let q_embed = (q.clone() * cos.clone()) + (Self::rotate_half(q) * sin.clone());
        let k_embed = (k.clone() * cos) + (Self::rotate_half(k) * sin);
        (q_embed, k_embed)
    }

    fn get_sincos<Seq: Dim, D: Device<f32> + Device<E> + Arange<f32>>(
        &self,
        device: &D,
        seq: Seq,
    ) -> (
        Tensor<(Seq, Const<HEAD_DIM>), E, D>,
        Tensor<(Seq, Const<HEAD_DIM>), E, D>,
    ) {
        let inv_freq = self.inv_freq.load_on(device);
        let t = device.arange(seq);
        let freqs = t.matmul(inv_freq);
        let emb = {
            // Implements `emb = torch.cat((freqs, freqs), dim=-1).to(x.device)`
            let freqs = freqs.permute().realize::<(usize, usize)>().unwrap();
            let emb = freqs.clone().concat(freqs);
            emb.permute().realize::<(Seq, Const<HEAD_DIM>)>().unwrap()
        };

        let emb_sin = emb.clone().sin();
        let emb_cos = emb.cos();
        (emb_sin.to_dtype::<E>(), emb_cos.to_dtype::<E>())
    }

    fn rotate_half<Batch: Dim, Seq: Dim, D: Device<E>>(
        x: Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D>,
    ) -> Tensor<(Batch, Const<NUM_HEADS>, Seq, Const<HEAD_DIM>), E, D> {
        let x1 = x.clone().slice((.., .., .., ..HEAD_DIM_OVER_2));
        let x2 = x.clone().slice((.., .., .., HEAD_DIM_OVER_2..));
        let x1 = x1.permute::<_, Axes4<3, 0, 1, 2>>();
        let x2 = x2.permute::<_, Axes4<3, 0, 1, 2>>();
        let y = x1.concat(x2).permute::<_, Axes4<1, 2, 3, 0>>();
        y.realize().unwrap()
    }
}

struct Attention {
    q_proj: LazyTensor<Rank2<HIDDEN, HIDDEN>, E>,
    k_proj: LazyTensor<Rank2<HIDDEN, HIDDEN>, E>,
    v_proj: LazyTensor<Rank2<HIDDEN, HIDDEN>, E>,
    out_proj: LazyTensor<Rank2<HIDDEN, HIDDEN>, E>,
    rotary_embed: RotaryEmbedding,
}

impl Attention {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E> + Device<f32>>(
        &self,
        x: Tensor<(Batch, Seq, Const<HIDDEN>), E, D>,
    ) -> Tensor<(Batch, Seq, Const<HIDDEN>), E, D> {
        let q_proj = self.q_proj.load_on(x.device());
        let k_proj = self.k_proj.load_on(x.device());
        let v_proj = self.v_proj.load_on(x.device());
        let out_proj = self.out_proj.load_on(x.device());

        let (batch, seq, _) = *x.shape();

        let q = x
            .clone()
            .matmul(q_proj)
            .reshape_like(&(batch, seq, Const::<NUM_HEADS>, Const::<HEAD_DIM>))
            .unwrap()
            .permute::<_, Axes4<0, 2, 1, 3>>();
        let k = x
            .clone()
            .matmul(k_proj)
            .reshape_like(&(batch, seq, Const::<NUM_HEADS>, Const::<HEAD_DIM>))
            .unwrap()
            .permute::<_, Axes4<0, 2, 1, 3>>();
        let v = x
            .matmul(v_proj)
            .reshape_like(&(batch, seq, Const::<NUM_HEADS>, Const::<HEAD_DIM>))
            .unwrap()
            .permute::<_, Axes4<0, 2, 1, 3>>();

        let (q, k) = self.rotary_embed.forward(q, k);

        let inv_head_scale: f32 = (HEAD_DIM as f32).sqrt().recip();
        let attn_weights = q.matmul(k.permute::<_, Axes4<0, 1, 3, 2>>()) * inv_head_scale;
        let attn_weights = attn_weights.softmax::<Axis<3>>();
        let attn_output = attn_weights.matmul(v);

        let attn_output = attn_output
            .permute::<_, Axes4<0, 2, 1, 3>>()
            .reshape_like(&(batch, seq, Const))
            .unwrap();

        attn_output.matmul(out_proj)
    }
}

struct MLP {
    gate_proj: LazyTensor<Rank2<HIDDEN, INTERMEDIATE>, E>,
    down_proj: LazyTensor<Rank2<INTERMEDIATE, HIDDEN>, E>,
    up_proj: LazyTensor<Rank2<HIDDEN, INTERMEDIATE>, E>,
}

impl MLP {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E>>(
        &self,
        x: Tensor<(Batch, Seq, Const<HIDDEN>), E, D>,
    ) -> Tensor<(Batch, Seq, Const<HIDDEN>), E, D> {
        let gate_proj = self.gate_proj.load_on(x.device());
        let down_proj = self.down_proj.load_on(x.device());
        let up_proj = self.up_proj.load_on(x.device());
        let gate = x.clone().matmul(gate_proj);
        let silu = gate.clone() * gate.sigmoid();
        let up = x.matmul(up_proj);
        (up * silu).matmul(down_proj)
    }
}

struct DecoderLayer {
    self_attn: Attention,
    mlp: MLP,
    input_layer_norm: RMSNorm,
    post_attention_layer_norm: RMSNorm,
}

impl DecoderLayer {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E> + Device<f32>>(
        &self,
        x: Tensor<(Batch, Seq, Const<HIDDEN>), E, D>,
    ) -> Tensor<(Batch, Seq, Const<HIDDEN>), E, D> {
        let residual = x.clone();
        let x = self.input_layer_norm.forward(x);
        let x = residual + self.self_attn.forward(x);
        let residual = x.clone();
        let x = self.post_attention_layer_norm.forward(x);
        residual + self.mlp.forward(x)
    }
}

struct Llama {
    embed_tokens: LazyTensor<Rank2<VOCAB, HIDDEN>, E>,
    layers: Vec<DecoderLayer>,
    norm: RMSNorm,
}

impl Llama {
    fn forward<Batch: Dim, Seq: Dim, D: Device<E> + Device<f32>>(
        &self,
        input_ids: Tensor<(Batch, Seq), usize, D>,
    ) -> Tensor<(Batch, Seq, Const<HIDDEN>), E, D> {
        let embed_tokens = self.embed_tokens.load_on(input_ids.device());
        let mut hidden_states = embed_tokens.gather(input_ids);
        for layer in self.layers.iter() {
            hidden_states = layer.forward(hidden_states);
        }
        self.norm.forward(hidden_states)
    }
}

pub struct LlamaForCausalLM {
    llama: Llama,
    lm_head: LazyTensor<Rank2<HIDDEN, VOCAB>, E>,
}

impl LlamaForCausalLM {
    pub fn forward<Batch: Dim, Seq: Dim, D: Device<E> + Device<f32>>(
        &self,
        input_ids: Tensor<(Batch, Seq), usize, D>,
    ) -> Tensor<(Batch, Seq, Const<VOCAB>), E, D> {
        let lm_head = self.lm_head.load_on(input_ids.device());
        let hidden_states = self.llama.forward(input_ids);
        hidden_states.matmul(lm_head)
    }
}
