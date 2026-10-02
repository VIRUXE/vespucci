import ctypes, sys
d3d = ctypes.windll.LoadLibrary("d3dcompiler_47.dll")
class Blob(ctypes.Structure): pass
data = open(sys.argv[1],'rb').read()
out = ctypes.c_void_p()
hr = d3d.D3DDisassemble(data, len(data), 0, None, ctypes.byref(out))
if hr != 0: sys.exit(f"HRESULT {hr:#x}")
# ID3DBlob vtable: QueryInterface, AddRef, Release, GetBufferPointer, GetBufferSize
vt = ctypes.cast(out, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p)))[0]
GetBufferPointer = ctypes.WINFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p)(vt[3])
GetBufferSize = ctypes.WINFUNCTYPE(ctypes.c_size_t, ctypes.c_void_p)(vt[4])
print(ctypes.string_at(GetBufferPointer(out), GetBufferSize(out)).decode())
