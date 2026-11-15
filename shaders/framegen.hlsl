// FrameForge frame generation: hierarchical block motion estimation + interpolation / extrapolation.
Texture2D<float4> Prev     : register(t0);
Texture2D<float4> Cur      : register(t1);
Texture2D<float4> MVIn     : register(t2);
Texture2D<float>  LCur     : register(t3);
Texture2D<float>  LPrev    : register(t4);
SamplerState SamLin        : register(s0);
RWTexture2D<float>  LumaOut : register(u0);
RWTexture2D<float4> MVOut   : register(u1);
RWTexture2D<float4> GenOut  : register(u2);

cbuffer FG : register(b0)
{
    uint2 src_size;
    uint2 luma_size;
    uint2 mv_size;
    uint2 init_size;
    uint  ds;
    uint  search;
    uint  use_init;
    uint  pad0;
    float t;
    float thresh;
    float pad1;
    float pad2;
};

float luma3(float3 c) { return dot(c, float3(0.299, 0.587, 0.114)); }

// color -> luma at 1/ds resolution (dispatch over luma_size)
[numthreads(8, 8, 1)]
void CSLuma(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= luma_size.x || id.y >= luma_size.y) return;
    float acc = 0;
    int2 o = int2(id.xy) * int(ds);
    int2 mx = int2(src_size) - int2(1, 1);
    [loop] for (uint j = 0; j < ds; j++)
    {
        [loop] for (uint i = 0; i < ds; i++)
        {
            int2 p = min(o + int2(i, j), mx);
            acc += luma3(Cur.Load(int3(p, 0)).rgb);
        }
    }
    LumaOut[id.xy] = acc / float(ds * ds);
}

// fine luma -> coarse luma (2x2 box); dispatch over coarse size
[numthreads(8, 8, 1)]
void CSLumaDown(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= luma_size.x || id.y >= luma_size.y) return;
    uint w, h;
    LCur.GetDimensions(w, h);
    int2 lim = int2(w, h) - int2(1, 1);
    int2 o = int2(id.xy) * 2;
    float a = 0;
    [unroll] for (int j = 0; j < 2; j++)
    {
        [unroll] for (int i = 0; i < 2; i++)
        {
            a += LCur.Load(int3(min(o + int2(i, j), lim), 0));
        }
    }
    LumaOut[id.xy] = a * 0.25;
}

// block matching: 4x4 blocks, 8x8 patch, search +-search around optional init vector
[numthreads(8, 8, 1)]
void CSMotion(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= mv_size.x || id.y >= mv_size.y) return;
    int2 lsz = int2(luma_size);
    int2 lim = lsz - int2(1, 1);
    int2 b0 = int2(id.xy) * 4 - int2(2, 2);
    int2 init = int2(0, 0);
    if (use_init != 0)
    {
        int2 cell = min(int2(id.xy >> 1), int2(init_size) - int2(1, 1));
        float2 v = MVIn.Load(int3(cell, 0)).xy * 2.0;
        init = int2(round(v));
    }
    int R = int(search);
    float best = 1e9;
    int2 bd = int2(0, 0);
    [loop] for (int dy = -R; dy <= R; dy++)
    {
        [loop] for (int dx = -R; dx <= R; dx++)
        {
            int2 d = init + int2(dx, dy);
            float sad = 0;
            [loop] for (int k = 0; k < 64; k++)
            {
                int2 o = int2(k & 7, k >> 3);
                float a = LCur.Load(int3(clamp(b0 + o, int2(0, 0), lim), 0));
                float b = LPrev.Load(int3(clamp(b0 + o + d, int2(0, 0), lim), 0));
                sad += abs(a - b);
            }
            sad += 0.01 * float(abs(d.x) + abs(d.y));
            if (sad < best) { best = sad; bd = d; }
        }
    }
    MVOut[id.xy] = float4(float2(bd), best, 0);
}

// vector median filter over 3x3
[numthreads(8, 8, 1)]
void CSSmooth(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= mv_size.x || id.y >= mv_size.y) return;
    int2 lim = int2(mv_size) - int2(1, 1);
    float2 v[9];
    [unroll] for (int k = 0; k < 9; k++)
    {
        int2 q = clamp(int2(id.xy) + int2(k % 3 - 1, k / 3 - 1), int2(0, 0), lim);
        v[k] = MVIn.Load(int3(q, 0)).xy;
    }
    float bestc = 1e9;
    float2 bv = v[4];
    [unroll] for (int a = 0; a < 9; a++)
    {
        float c = 0;
        [unroll] for (int b = 0; b < 9; b++) c += length(v[a] - v[b]);
        if (c < bestc) { bestc = c; bv = v[a]; }
    }
    MVOut[id.xy] = float4(bv, 0, 0);
}

float2 fetch_mv(float2 p)
{
    float2 uv = p / (float(ds) * 4.0) / float2(mv_size);
    return MVIn.SampleLevel(SamLin, uv, 0).xy * float(ds);
}

// t in (0,1]: frame between Prev (t=0) and Cur (t=1)
[numthreads(8, 8, 1)]
void CSInterp(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= src_size.x || id.y >= src_size.y) return;
    float2 p = float2(id.xy) + 0.5;
    float2 sz = float2(src_size);
    float2 mv = fetch_mv(p);
    float3 a = Prev.SampleLevel(SamLin, (p + mv * t) / sz, 0).rgb;
    float3 b = Cur.SampleLevel(SamLin, (p - mv * (1.0 - t)) / sz, 0).rgb;
    float3 a0 = Prev.Load(int3(id.xy, 0)).rgb;
    float3 b0 = Cur.Load(int3(id.xy, 0)).rgb;
    float err = length(a - b);
    float3 warped = lerp(a, b, t);
    float3 fallback = (t < 0.5) ? a0 : b0;
    float w = smoothstep(thresh, thresh * 2.5, err);
    GenOut[id.xy] = float4(lerp(warped, fallback, w), 1);
}

// t in (0,1): Cur advanced by t of the last motion (zero added latency)
[numthreads(8, 8, 1)]
void CSExtrap(uint3 id : SV_DispatchThreadID)
{
    if (id.x >= src_size.x || id.y >= src_size.y) return;
    float2 p = float2(id.xy) + 0.5;
    float2 sz = float2(src_size);
    float2 mv = fetch_mv(p);
    float3 c0 = Cur.Load(int3(id.xy, 0)).rgb;
    float3 c = Cur.SampleLevel(SamLin, (p + mv * t) / sz, 0).rgb;
    float3 pv = Prev.SampleLevel(SamLin, (p + mv) / sz, 0).rgb;
    float resid = length(pv - c0);
    float w = smoothstep(thresh, thresh * 2.5, resid);
    GenOut[id.xy] = float4(lerp(c, c0, w), 1);
}
