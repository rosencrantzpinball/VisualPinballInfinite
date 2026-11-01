// Pictures standing in the room (room.rs), ours: the headset's panel and the pause menu drawn into an
// eye's picture as quads in the room, each from its picture's mipmaps, its edges smoothed by how much of
// each pixel the picture covers.
//
// Registers follow SDL3 GPU's rules (see player.hlsl). The quad's corners come from the vertex number.

cbuffer RoomQuad : register(b0, space1)
{
    // the quad's corners in the eye's clip space, a margin beyond its edges included: top left, top
    // right, bottom left, bottom right
    float4 room_corners[4];
    // where the corners are on the picture: x and y at the top left, z and w at the bottom right
    // (below 0 and above 1 by the margin)
    float4 room_uv;
};

struct RoomVaryings
{
    float4 clip : SV_Position;
    // where on the picture, 0 to 1 within its edges
    float2 uv : TEXCOORD0;
};

static const uint room_order[6] = { 0, 1, 2, 1, 3, 2 };

RoomVaryings room_vs(uint id : SV_VertexID)
{
    uint c = room_order[id % 6];
    float2 t = float2(c & 1, c >> 1);
    RoomVaryings o;
    o.clip = room_corners[c];
    o.uv = lerp(room_uv.xy, room_uv.zw, t);
    return o;
}

Texture2D<float4> room_picture : register(t0, space2);
SamplerState room_sampler : register(s0, space2);

cbuffer RoomLook : register(b0, space3)
{
    // x: 1 to take the picture's alpha (straight, the pause menu's corners see through), 0 for a
    // picture shown whole; y: 1 when the target holds sRGB values (the picture's colours come linear)
    float4 room_look;
};

float3 room_to_srgb(float3 c)
{
    return c <= 0.0031308 ? c * 12.92 : 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}

float4 room_fs(RoomVaryings v) : SV_Target0
{
    float4 c = room_picture.Sample(room_sampler, v.uv);
    // how far inside the picture's edges the pixel's centre is, in pixels, and a half more: the share of
    // the pixel the picture covers, nearly
    float2 step = max(fwidth(v.uv), 1e-6);
    float2 inside = min(v.uv, 1.0 - v.uv) / step;
    float a = saturate(min(inside.x, inside.y) + 0.5);
    if (room_look.x > 0.5)
        a *= c.a;
    if (room_look.y > 0.5)
        c.rgb = room_to_srgb(c.rgb);
    return float4(c.rgb, a);
}
