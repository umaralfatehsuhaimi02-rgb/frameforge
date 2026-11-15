// FrameForge present shaders: upscalers (pass 1) + sharpen/effects/OSD (pass 2).
Texture2D<float4> Src : register(t0);
SamplerState SamLin : register(s0);
SamplerState SamPt  : register(s1);

cbuffer CB : register(b0)
{
    float2 src_size;
    float2 dst_size;
    float4 dst_rect;   // x0,y0,x1,y1 in destination pixels
    uint   mode;
    float  sharp;
    float  scanlines;
    float  saturation;
    float  contrast;
    uint   osd_on;
    float2 pad;
    uint4  osd0;
    uint4  osd1;
};

struct VSOut { float4 pos : SV_Position; float2 uv : TEXCOORD0; };

VSOut VSMain(uint id : SV_VertexID)
{
    VSOut o;
    float2 p = float2((id << 1) & 2, id & 2);
    o.uv = p;
    o.pos = float4(p * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
    return o;
}

float luma3(float3 c) { return dot(c, float3(0.299, 0.587, 0.114)); }

float3 ld(int2 c)
{
    c = clamp(c, int2(0, 0), int2(src_size) - int2(1, 1));
    return Src.Load(int3(c, 0)).rgb;
}

// tc is in texel units (texel centres at +0.5)
float3 smp(float2 tc) { return Src.SampleLevel(SamLin, tc / src_size, 0).rgb; }

float3 bicubic(float2 tc)
{
    float2 tc1 = floor(tc - 0.5) + 0.5;
    float2 f = tc - tc1;
    float2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    float2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    float2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    float2 w3 = f * f * (-0.5 + 0.5 * f);
    float2 w12 = w1 + w2;
    float2 tc0 = tc1 - 1.0;
    float2 tc3 = tc1 + 2.0;
    float2 tc12 = tc1 + w2 / w12;
    float3 c = 0;
    c += smp(float2(tc0.x,  tc0.y))  * (w0.x  * w0.y);
    c += smp(float2(tc12.x, tc0.y))  * (w12.x * w0.y);
    c += smp(float2(tc3.x,  tc0.y))  * (w3.x  * w0.y);
    c += smp(float2(tc0.x,  tc12.y)) * (w0.x  * w12.y);
    c += smp(float2(tc12.x, tc12.y)) * (w12.x * w12.y);
    c += smp(float2(tc3.x,  tc12.y)) * (w3.x  * w12.y);
    c += smp(float2(tc0.x,  tc3.y))  * (w0.x  * w3.y);
    c += smp(float2(tc12.x, tc3.y))  * (w12.x * w3.y);
    c += smp(float2(tc3.x,  tc3.y))  * (w3.x  * w3.y);
    return max(c, 0.0);
}

float lz(float x)
{
    x = abs(x);
    if (x < 1e-4) return 1.0;
    if (x >= 3.0) return 0.0;
    float px = 3.14159265 * x;
    return 3.0 * sin(px) * sin(px / 3.0) / (px * px);
}

float3 lanczos(float2 tc)
{
    int2 base = int2(floor(tc - 0.5));
    float3 sum = 0;
    float wsum = 0;
    [loop] for (int j = -2; j <= 3; j++)
    {
        [loop] for (int i = -2; i <= 3; i++)
        {
            int2 ip = base + int2(i, j);
            float2 d = (float2(ip) + 0.5) - tc;
            float w = lz(d.x) * lz(d.y);
            sum += ld(ip) * w;
            wsum += w;
        }
    }
    return saturate(sum / max(wsum, 1e-5));
}

float3 sharp_bilinear(float2 tc, float2 scale)
{
    scale = max(scale, float2(1.0, 1.0));
    float2 fl = floor(tc);
    float2 s = tc - fl;
    float2 region = 0.5 - 0.5 / scale;
    float2 cd = s - 0.5;
    float2 f = (cd - clamp(cd, -region, region)) * scale + 0.5;
    return smp(fl + f);
}

float3 edge_adaptive(float2 tc)
{
    int2 ip = int2(floor(tc - 0.5));
    float3 c00 = ld(ip);
    float3 c10 = ld(ip + int2(1, 0));
    float3 c01 = ld(ip + int2(0, 1));
    float3 c11 = ld(ip + int2(1, 1));
    float l00 = luma3(c00), l10 = luma3(c10), l01 = luma3(c01), l11 = luma3(c11);
    float gx = (l10 + l11) - (l00 + l01);
    float gy = (l01 + l11) - (l00 + l10);
    float g = length(float2(gx, gy));
    float3 base = bicubic(tc);
    if (g < 0.02) return base;
    float2 dir = float2(-gy, gx) / g; // along the edge
    float3 s = base * 0.5 + (bicubic(tc + dir * 0.6) + bicubic(tc - dir * 0.6)) * 0.25;
    float3 mn = min(min(c00, c10), min(c01, c11));
    float3 mx = max(max(c00, c10), max(c01, c11));
    float k = saturate(g * 4.0);
    return lerp(s, clamp(s, mn, mx), k);
}

float3 anime(float2 tc)
{
    float3 base = bicubic(tc);
    int2 c = int2(floor(tc));
    float3 mn = float3(1, 1, 1);
    float3 mx = float3(0, 0, 0);
    [unroll] for (int j = -1; j <= 1; j++)
    {
        [unroll] for (int i = -1; i <= 1; i++)
        {
            float3 v = ld(c + int2(i, j));
            mn = min(mn, v);
            mx = max(mx, v);
        }
    }
    float lb = luma3(base), lmn = luma3(mn), lmx = luma3(mx);
    float edge = saturate((lmx - lmn) * 2.0);
    float3 dark = lerp(base, mn, 0.35 * edge * saturate((lmx - lb) * 4.0));
    float3 blur = (smp(tc + float2(1, 0)) + smp(tc - float2(1, 0)) + smp(tc + float2(0, 1)) + smp(tc - float2(0, 1))) * 0.25;
    float3 sh = dark + (dark - blur) * 0.8;
    return clamp(sh, mn, mx);
}

float4 PSScale(VSOut i) : SV_Target
{
    float2 rs = dst_rect.zw - dst_rect.xy;
    float2 q = (i.pos.xy - dst_rect.xy) / rs;
    if (q.x < 0.0 || q.y < 0.0 || q.x >= 1.0 || q.y >= 1.0) return float4(0, 0, 0, 1);
    float2 tc = q * src_size;
    float3 c;
    if (mode == 0)      c = ld(int2(floor(tc)));
    else if (mode == 1) c = smp(tc);
    else if (mode == 2) c = sharp_bilinear(tc, rs / src_size);
    else if (mode == 4) c = bicubic(tc);
    else if (mode == 5) c = lanczos(tc);
    else if (mode == 6) c = edge_adaptive(tc);
    else                c = anime(tc);
    return float4(saturate(c), 1);
}

static const uint FONT[12] = { 0x7B6F, 0x2C97, 0x73E7, 0x73CF, 0x5BC9, 0x79CF, 0x79EF, 0x7249, 0x7BEF, 0x7BCF, 0x12A4, 0 };

float4 PSPost(VSOut i) : SV_Target
{
    int2 p = int2(i.pos.xy);
    int2 lim = int2(dst_size) - int2(1, 1);
    float3 e = Src.Load(int3(p, 0)).rgb;
    float3 c = e;
    bool inside = i.pos.x >= dst_rect.x && i.pos.x < dst_rect.z && i.pos.y >= dst_rect.y && i.pos.y < dst_rect.w;
    if (inside)
    {
        if (sharp > 0.0)
        {
            float3 b = Src.Load(int3(clamp(p + int2(0, -1), int2(0, 0), lim), 0)).rgb;
            float3 d = Src.Load(int3(clamp(p + int2(-1, 0), int2(0, 0), lim), 0)).rgb;
            float3 f = Src.Load(int3(clamp(p + int2(1, 0), int2(0, 0), lim), 0)).rgb;
            float3 h = Src.Load(int3(clamp(p + int2(0, 1), int2(0, 0), lim), 0)).rgb;
            float3 mn = min(min(min(d, e), min(f, b)), h);
            float3 mxv = max(max(max(d, e), max(f, b)), h);
            float3 amp = saturate(min(mn, 1.0 - mxv) / max(mxv, float3(1e-4, 1e-4, 1e-4)));
            amp = sqrt(amp);
            float peak = -1.0 / lerp(8.0, 5.0, sharp);
            float3 w = amp * peak;
            float3 rcpw = 1.0 / (1.0 + 4.0 * w);
            c = saturate((b * w + d * w + f * w + h * w + e) * rcpw);
        }
        float l = luma3(c);
        c = lerp(float3(l, l, l), c, saturation);
        c = (c - 0.5) * contrast + 0.5;
        if (scanlines > 0.0)
        {
            float sy = (i.pos.y - dst_rect.y) / (dst_rect.w - dst_rect.y) * src_size.y;
            float s = 0.5 + 0.5 * cos(6.2831853 * sy);
            c *= 1.0 - scanlines * 0.7 * s;
        }
    }
    if (osd_on != 0)
    {
        int2 o = p - int2(16, 16);
        if (o.x >= -4 && o.y >= -4 && o.x < 132 && o.y < 24) c *= 0.35;
        if (o.x >= 0 && o.y >= 0)
        {
            int2 g = o / 4;
            int ci = g.x / 4;
            int cx = g.x % 4;
            int cy = g.y;
            if (ci < 8 && cx < 3 && cy < 5)
            {
                uint code = (ci < 4) ? osd0[ci] : osd1[max(ci - 4, 0)];
                code = min(code, 11u);
                uint bits = FONT[code];
                if (((bits >> uint(14 - (cy * 3 + cx))) & 1u) != 0u) c = float3(0.25, 1.0, 0.3);
            }
        }
    }
    return float4(saturate(c), 1);
}
