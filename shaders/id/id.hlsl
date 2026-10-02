// Vespucci helper: draw-id pixel shader for picking. It pairs with any of the
// game's vertex shaders (it reads none of their outputs) and writes the id from
// its own constant buffer to an R32_UINT target.
//   vkd3d-compiler -b dxbc-tpf -p ps_5_0 -e main shaders/id/id.hlsl -o shaders/id/id.ps.dxbc
cbuffer VespucciId : register(b0)
{
    uint4 gId;
};

uint main() : SV_Target0
{
    return gId.x;
}
