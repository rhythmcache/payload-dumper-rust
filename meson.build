project('payload_dumper', 'c',
  version : '1.0.0',
  meson_version : '>= 1.1.0',
  default_options : [
    'c_std=c11',
    'warning_level=3',
    'werror=false'
  ]
)

cc = meson.get_compiler('c')

# liblzma
lzma_dep = dependency('liblzma', required: false, default_options: ['default_library=static'])
if not lzma_dep.found()
  lzma_dep = dependency('liblzma', fallback: ['liblzma', 'lzma_dep'], default_options: ['default_library=static'])
endif

# libzstd
zstd_dep = dependency('libzstd', required: false, default_options: ['default_library=static'])
if not zstd_dep.found()
  zstd_dep = dependency('libzstd', fallback: ['zstd', 'libzstd_dep'], default_options: ['default_library=static'])
endif

# bzip2
bz2_dep = dependency('bzip2', required: false, default_options: ['default_library=static'])
if not bz2_dep.found()
  bz2_dep = dependency('bzip2', fallback: ['bzip2', 'bzip2_dep'], default_options: ['default_library=static'])
endif

# libprotobuf-c
protobuf_c_dep = dependency('libprotobuf-c', required: false, default_options: ['default_library=static'])
if not protobuf_c_dep.found()
  protobuf_c_dep = dependency('libprotobuf-c', fallback: ['protobuf-c', 'libprotobuf_c_dep'], default_options: ['default_library=static'])
endif

deps = [
  lzma_dep,
  zstd_dep,
  bz2_dep,
  protobuf_c_dep
]

if host_machine.system() != 'android'
  threads_dep = dependency('threads')
  deps += threads_dep
endif

# libcurl (optional)
curl_dep = dependency('libcurl', required: false, default_options: ['default_library=static'])
if curl_dep.found()
  http_enabled = true
  deps += curl_dep
else
  http_enabled = false
  message('Building without HTTP support')
endif

protoc_c = find_program('protoc-c', required: false)

fs = import('fs')
proto_exists = fs.exists('proto/update_metadata.proto')

if protoc_c.found() and proto_exists
  update_metadata_pb = custom_target('update_metadata_pb',
    output: ['update_metadata.pb-c.c', 'update_metadata.pb-c.h'],
    input: 'proto/update_metadata.proto',
    command: [protoc_c, '--proto_path=@SOURCE_ROOT@/proto', '--c_out=@OUTDIR@', 'update_metadata.proto'],
    depend_files: 'proto/update_metadata.proto'
  )
  pb_sources = [update_metadata_pb[0]]
  pb_inc = include_directories('.', 'proto')
else
  pb_sources = files('proto/update_metadata.pb-c.c')
  pb_inc = include_directories('proto')
endif

sources = [
  'src/payload_dumper.c',
  'src/zip/zip_parser.c',
  'src/zip/zip_parser.h'
] + pb_sources

if http_enabled
  sources += ['src/http/http_reader.c', 'src/http/http_reader.h']
endif

compile_args = []
if http_enabled
  compile_args += '-DENABLE_HTTP_SUPPORT'
endif

executable('payload_dumper',
  sources,
  dependencies: deps,
  c_args: compile_args,
  include_directories: [
    include_directories('src'),
    include_directories('src/zip'),
    include_directories('src/http'),
    pb_inc
  ],
  install: true,
  install_dir: get_option('bindir')
)