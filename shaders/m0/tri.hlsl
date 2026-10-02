// M0 spike: one clip-space triangle, solid colour. Compiled with vkd3d-compiler to DXBC.
struct VSIn  { float3 pos : POSITION; float4 col : COLOR0; };
struct VSOut { float4 pos : SV_Position; float4 col : COLOR0; };
VSOut vs_main(VSIn i) { VSOut o; o.pos = float4(i.pos, 1.0); o.col = i.col; return o; }
float4 ps_main(VSOut i) : SV_Target { return i.col; }
