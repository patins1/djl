use crate::layers::{LayerNorm, Linear};
use crate::models::Model;
use candle::{Device, IndexOp, Result, Tensor};
use candle_nn::{embedding, Embedding, Module, VarBuilder};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HiddenAct {
    Gelu,
    GeluApproximate,
    Relu,
}

struct HiddenActLayer {
    act: HiddenAct,
    span: tracing::Span,
}

impl HiddenActLayer {
    fn new(act: HiddenAct) -> Self {
        let span = tracing::span!(tracing::Level::TRACE, "hidden-act");
        Self { act, span }
    }

    fn forward(&self, xs: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        match self.act {
            // https://github.com/huggingface/transformers/blob/cd4584e3c809bb9e1392ccd3fe38b40daba5519a/src/transformers/activations.py#L213
            HiddenAct::Gelu => xs.gelu_erf(),
            HiddenAct::GeluApproximate => xs.gelu(),
            HiddenAct::Relu => xs.relu(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PositionEmbeddingType {
    #[default]
    Absolute,
    Alibi,
    Rope,
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/configuration_bert.py#L1
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BertConfig {
    pub architectures: Vec<String>,
    model_type: Option<String>,
    vocab_size: usize,
    hidden_size: usize,
    num_hidden_layers: usize,
    num_attention_heads: usize,
    intermediate_size: usize,
    pub hidden_act: HiddenAct,
    hidden_dropout_prob: f64,
    max_position_embeddings: usize,
    type_vocab_size: usize,
    initializer_range: f64,
    layer_norm_eps: f64,
    pad_token_id: usize,
    #[serde(default)]
    position_embedding_type: PositionEmbeddingType,
    #[serde(default)]
    use_cache: bool,
    classifier_dropout: Option<f64>,
    pub id2label: Option<HashMap<String, String>>,
    pub use_flash_attn: Option<bool>,
}

impl Default for BertConfig {
    fn default() -> Self {
        Self {
            architectures: Vec::new(),
            model_type: Some("bert".to_string()),
            vocab_size: 30522,
            hidden_size: 768,
            num_hidden_layers: 12,
            num_attention_heads: 12,
            intermediate_size: 3072,
            hidden_act: HiddenAct::Gelu,
            hidden_dropout_prob: 0.1,
            max_position_embeddings: 512,
            type_vocab_size: 2,
            initializer_range: 0.02,
            layer_norm_eps: 1e-12,
            pad_token_id: 0,
            position_embedding_type: PositionEmbeddingType::Absolute,
            use_cache: true,
            classifier_dropout: None,
            id2label: None,
            use_flash_attn: Some(false),
        }
    }
}

struct Dropout {
    #[allow(dead_code)]
    pr: f64,
}

impl Dropout {
    fn new(pr: f64) -> Self {
        Self { pr }
    }
}

impl Module for Dropout {
    fn forward(&self, x: &Tensor) -> Result<Tensor> {
        // TODO
        Ok(x.clone())
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L180
struct BertEmbeddings {
    word_embeddings: Embedding,
    position_embeddings: Option<Embedding>,
    token_type_embeddings: Embedding,
    layer_norm: LayerNorm,
    dropout: Dropout,
    span: tracing::Span,
}

impl BertEmbeddings {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let word_embeddings = embedding(
            config.vocab_size,
            config.hidden_size,
            vb.pp("word_embeddings"),
        )?;
        let position_embeddings = embedding(
            config.max_position_embeddings,
            config.hidden_size,
            vb.pp("position_embeddings"),
        )?;
        let token_type_embeddings = embedding(
            config.type_vocab_size,
            config.hidden_size,
            vb.pp("token_type_embeddings"),
        )?;
        let layer_norm = LayerNorm::load(
            vb.pp("LayerNorm"),
            config.hidden_size,
            config.layer_norm_eps as f32,
        )?;
        Ok(Self {
            word_embeddings,
            position_embeddings: Some(position_embeddings),
            token_type_embeddings,
            layer_norm,
            dropout: Dropout::new(config.hidden_dropout_prob),
            span: tracing::span!(tracing::Level::TRACE, "embeddings"),
        })
    }

    fn forward(&self, input_ids: &Tensor, token_type_ids: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let (_bsize, seq_len) = input_ids.dims2()?;
        let input_embeddings = self.word_embeddings.forward(input_ids)?;
        let token_type_embeddings = self.token_type_embeddings.forward(token_type_ids)?;
        let mut embeddings = (&input_embeddings + token_type_embeddings)?;
        if let Some(position_embeddings) = &self.position_embeddings {
            // TODO: Proper absolute positions?
            let position_ids = (0..seq_len as u32).collect::<Vec<_>>();
            let position_ids = Tensor::new(&position_ids[..], input_ids.device())?;
            embeddings = embeddings.broadcast_add(&position_embeddings.forward(&position_ids)?)?
        }
        let embeddings = self.layer_norm.forward(&embeddings, None)?;
        let embeddings = self.dropout.forward(&embeddings)?;
        Ok(embeddings)
    }
}

struct BertSelfAttention {
    query: Linear,
    key: Linear,
    value: Linear,
    dropout: Dropout,
    num_attention_heads: usize,
    attention_head_size: usize,
    use_flash_attn: bool,
    span: tracing::Span,
    span_softmax: tracing::Span,
}

impl BertSelfAttention {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let attention_head_size = config.hidden_size / config.num_attention_heads;
        let all_head_size = config.num_attention_heads * attention_head_size;
        let hidden_size = config.hidden_size;
        let dropout = Dropout::new(config.hidden_dropout_prob);
        let query = Linear::load(vb.pp("query"), hidden_size, all_head_size, None)?;
        let value = Linear::load(vb.pp("value"), hidden_size, all_head_size, None)?;
        let key = Linear::load(vb.pp("key"), hidden_size, all_head_size, None)?;
        Ok(Self {
            query,
            key,
            value,
            dropout,
            num_attention_heads: config.num_attention_heads,
            attention_head_size,
            use_flash_attn: config.use_flash_attn.unwrap_or(false),
            span: tracing::span!(tracing::Level::TRACE, "self-attn"),
            span_softmax: tracing::span!(tracing::Level::TRACE, "softmax"),
        })
    }

    fn transpose_for_scores(&self, xs: &Tensor) -> Result<Tensor> {
        let mut new_x_shape = xs.dims().to_vec();
        new_x_shape.pop();
        new_x_shape.push(self.num_attention_heads);
        new_x_shape.push(self.attention_head_size);
        let xs = xs.reshape(new_x_shape.as_slice())?.transpose(1, 2)?;
        xs.contiguous()
    }
}

#[cfg(feature = "flash-attn")]
fn flash_attn(
    q: &Tensor,
    k: &Tensor,
    v: &Tensor,
    softmax_scale: f32,
    causal: bool,
) -> Result<Tensor> {
    candle_flash_attn::flash_attn(q, k, v, softmax_scale, causal)
}

#[cfg(not(feature = "flash-attn"))]
fn flash_attn(_: &Tensor, _: &Tensor, _: &Tensor, _: f32, _: bool) -> Result<Tensor> {
    unimplemented!("compile with '--features flash-attn'")
}

impl Module for BertSelfAttention {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let query_layer = self.query.forward(hidden_states)?;
        let key_layer = self.key.forward(hidden_states)?;
        let value_layer = self.value.forward(hidden_states)?;

        let query_layer = self.transpose_for_scores(&query_layer)?;
        let key_layer = self.transpose_for_scores(&key_layer)?;
        let value_layer = self.transpose_for_scores(&value_layer)?;

        let context_layer = if self.use_flash_attn {
            // flash-attn expects (b_sz, seq_len, nheads, head_dim)
            let q = query_layer.transpose(1, 2)?;
            let k = key_layer.transpose(1, 2)?;
            let v = value_layer.transpose(1, 2)?;
            let softmax_scale = 1f32 / (self.attention_head_size as f32).sqrt();
            flash_attn(&q, &k, &v, softmax_scale, false)?.transpose(1, 2)?
        } else {
            let attention_scores = query_layer.matmul(&key_layer.t()?)?;
            let attention_scores = (attention_scores / (self.attention_head_size as f64).sqrt())?;
            let attention_probs = {
                let _enter_sm = self.span_softmax.enter();
                candle_nn::ops::softmax(&attention_scores, candle::D::Minus1)?
            };
            let attention_probs = self.dropout.forward(&attention_probs)?;

            let context_layer = attention_probs.matmul(&value_layer)?;
            context_layer
        };

        let context_layer = context_layer.transpose(1, 2)?.contiguous()?;
        let context_layer = context_layer.flatten_from(candle::D::Minus2)?;
        Ok(context_layer)
    }
}

struct BertSelfOutput {
    dense: Linear,
    layer_norm: LayerNorm,
    dropout: Dropout,
    span: tracing::Span,
}

impl BertSelfOutput {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let dense = Linear::load(vb.pp("dense"), config.hidden_size, config.hidden_size, None)?;
        let layer_norm = LayerNorm::load(
            vb.pp("LayerNorm"),
            config.hidden_size,
            config.layer_norm_eps as f32,
        )?;
        let dropout = Dropout::new(config.hidden_dropout_prob);
        Ok(Self {
            dense,
            layer_norm,
            dropout,
            span: tracing::span!(tracing::Level::TRACE, "self-out"),
        })
    }

    fn forward(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let hidden_states = self.dense.forward(hidden_states)?;
        let hidden_states = self.dropout.forward(&hidden_states)?;
        self.layer_norm.forward(&hidden_states, Some(&input_tensor))
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L392
struct BertAttention {
    self_attention: BertSelfAttention,
    self_output: BertSelfOutput,
    span: tracing::Span,
}

impl BertAttention {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let self_attention = BertSelfAttention::load(vb.pp("self"), config)?;
        let self_output = BertSelfOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            self_attention,
            self_output,
            span: tracing::span!(tracing::Level::TRACE, "attn"),
        })
    }
}

impl Module for BertAttention {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let self_outputs = self.self_attention.forward(hidden_states)?;
        let attention_output = self.self_output.forward(&self_outputs, hidden_states)?;
        Ok(attention_output)
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L441
struct BertIntermediate {
    dense: Linear,
    intermediate_act: HiddenActLayer,
    span: tracing::Span,
}

impl BertIntermediate {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let dense = Linear::load(
            vb.pp("dense"),
            config.hidden_size,
            config.intermediate_size,
            None,
        )?;
        Ok(Self {
            dense,
            intermediate_act: HiddenActLayer::new(config.hidden_act),
            span: tracing::span!(tracing::Level::TRACE, "inter"),
        })
    }
}

impl Module for BertIntermediate {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let hidden_states = self.dense.forward(hidden_states)?;
        let ys = self.intermediate_act.forward(&hidden_states)?;
        Ok(ys)
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L456
struct BertOutput {
    dense: Linear,
    layer_norm: LayerNorm,
    dropout: Dropout,
    span: tracing::Span,
}

impl BertOutput {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let dense = Linear::load(
            vb.pp("dense"),
            config.intermediate_size,
            config.hidden_size,
            None,
        )?;
        let layer_norm = LayerNorm::load(
            vb.pp("LayerNorm"),
            config.hidden_size,
            config.layer_norm_eps as f32,
        )?;
        let dropout = Dropout::new(config.hidden_dropout_prob);
        Ok(Self {
            dense,
            layer_norm,
            dropout,
            span: tracing::span!(tracing::Level::TRACE, "out"),
        })
    }

    fn forward(&self, hidden_states: &Tensor, input_tensor: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let hidden_states = self.dense.forward(hidden_states)?;
        let hidden_states = self.dropout.forward(&hidden_states)?;
        self.layer_norm.forward(&hidden_states, Some(&input_tensor))
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L470
struct BertLayer {
    attention: BertAttention,
    intermediate: BertIntermediate,
    output: BertOutput,
    span: tracing::Span,
}

impl BertLayer {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let attention = BertAttention::load(vb.pp("attention"), config)?;
        let intermediate = BertIntermediate::load(vb.pp("intermediate"), config)?;
        let output = BertOutput::load(vb.pp("output"), config)?;
        Ok(Self {
            attention,
            intermediate,
            output,
            span: tracing::span!(tracing::Level::TRACE, "layer"),
        })
    }
}

impl Module for BertLayer {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let attention_output = self.attention.forward(hidden_states)?;
        // TODO: Support cross-attention?
        // https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L523
        // TODO: Support something similar to `apply_chunking_to_forward`?
        let intermediate_output = self.intermediate.forward(&attention_output)?;
        let layer_output = self
            .output
            .forward(&intermediate_output, &attention_output)?;
        Ok(layer_output)
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L556
struct BertEncoder {
    layers: Vec<BertLayer>,
    span: tracing::Span,
}

impl BertEncoder {
    fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let layers = (0..config.num_hidden_layers)
            .map(|index| BertLayer::load(vb.pp(&format!("layer.{index}")), config))
            .collect::<Result<Vec<_>>>()?;
        let span = tracing::span!(tracing::Level::TRACE, "encoder");
        Ok(BertEncoder { layers, span })
    }
}

impl Module for BertEncoder {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();
        let mut hidden_states = hidden_states.clone();
        // Use a loop rather than a fold as it's easier to modify when adding debug/...
        for layer in self.layers.iter() {
            hidden_states = layer.forward(&hidden_states)?
        }
        Ok(hidden_states)
    }
}

pub trait ClassificationHead {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor>;
}

pub struct BertClassificationHead {
    pooler: Option<Linear>,
    output: Linear,
    span: tracing::Span,
}

impl BertClassificationHead {
    pub(crate) fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let n_classes = match &config.id2label {
            None => candle::bail!("`id2label` must be set for classifier models"),
            Some(id2label) => id2label.len(),
        };

        let pooler: Option<Linear> = match Linear::load(
            vb.pp("bert.pooler.dense"),
            config.hidden_size,
            config.hidden_size,
            None,
        ) {
            Ok(layer) => Some(layer),
            Err(_) => None,
        };
        let output = match Linear::load(vb.pp("classifier"), config.hidden_size, n_classes, None) {
            Ok(output) => output,
            Err(err) => {
                if let Ok(output) = Linear::load(vb, config.hidden_size, n_classes, None) {
                    output
                } else {
                    return Err(err);
                }
            }
        };

        Ok(Self {
            pooler,
            output,
            span: tracing::span!(tracing::Level::TRACE, "classifier"),
        })
    }
}

impl ClassificationHead for BertClassificationHead {
    fn forward(&self, hidden_states: &Tensor) -> Result<Tensor> {
        let _enter = self.span.enter();

        let mut hidden_states = hidden_states.unsqueeze(1)?;
        if let Some(pooler) = self.pooler.as_ref() {
            hidden_states = pooler.forward(&hidden_states)?;
            hidden_states = hidden_states.tanh()?;
        }

        let hidden_states = self.output.forward(&hidden_states)?;
        let hidden_states = hidden_states.squeeze(1)?;
        Ok(hidden_states)
    }
}

// https://github.com/huggingface/transformers/blob/6eedfa6dd15dc1e22a55ae036f681914e5a0d9a1/src/transformers/models/bert/modeling_bert.py#L874
pub struct BertModel {
    embeddings: BertEmbeddings,
    encoder: BertEncoder,
    #[allow(unused)]
    pub device: Device,
    span: tracing::Span,
}

impl BertModel {
    pub fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let (embeddings, encoder) = match (
            BertEmbeddings::load(vb.pp("embeddings"), config),
            BertEncoder::load(vb.pp("encoder"), config),
        ) {
            (Ok(embeddings), Ok(encoder)) => (embeddings, encoder),
            (Err(err), _) | (_, Err(err)) => {
                if let (Ok(embeddings), Ok(encoder)) = (
                    BertEmbeddings::load(vb.pp("bert.embeddings".to_string()), config),
                    BertEncoder::load(vb.pp("bert.encoder".to_string()), config),
                ) {
                    (embeddings, encoder)
                } else {
                    return Err(err);
                }
            }
        };
        Ok(Self {
            embeddings,
            encoder,
            device: vb.device().clone(),
            span: tracing::span!(tracing::Level::TRACE, "model"),
        })
    }
}

impl Model for BertModel {
    fn get_input_names(&self) -> Vec<String> {
        return vec![
            "input_ids".to_string(),
            "attention_mask".to_string(),
            "token_type_ids".to_string(),
        ];
    }

    fn forward(
        &self,
        input_ids: &Tensor,
        _attention_mask: &Tensor,
        token_type_ids: Option<&Tensor>,
    ) -> Result<Tensor> {
        let _enter = self.span.enter();
        let embedding_output = self
            .embeddings
            .forward(input_ids, token_type_ids.unwrap())?;
        let sequence_output = self.encoder.forward(&embedding_output)?;
        Ok(sequence_output)
    }
}

pub struct BertForSequenceClassification {
    bert: Box<BertModel>,
    classifier: Box<dyn ClassificationHead + Send>,
    #[allow(unused)]
    pub device: Device,
    span: tracing::Span,
}

impl BertForSequenceClassification {
    pub fn load(vb: VarBuilder, config: &BertConfig) -> Result<Self> {
        let bert = Box::new(BertModel::load(vb.clone(), &config)?);
        let classifier: Box<dyn ClassificationHead + Send> =
            Box::new(BertClassificationHead::load(vb.pp("classifier"), config)?);
        Ok(Self {
            bert,
            classifier,
            device: vb.device().clone(),
            span: tracing::span!(tracing::Level::TRACE, "model"),
        })
    }
}

impl Model for BertForSequenceClassification {
    fn get_input_names(&self) -> Vec<String> {
        return vec![
            "input_ids".to_string(),
            "attention_mask".to_string(),
            "token_type_ids".to_string(),
        ];
    }

    fn forward(
        &self,
        input_ids: &Tensor,
        attention_mask: &Tensor,
        token_type_ids: Option<&Tensor>,
    ) -> Result<Tensor> {
        let _enter = self.span.enter();
        let embeddings = self
            .bert
            .forward(input_ids, attention_mask, token_type_ids)?;
        let sequence_output = embeddings.i((.., 0))?;
        let logits = self.classifier.forward(&sequence_output)?;
        Ok(logits)
    }
}
