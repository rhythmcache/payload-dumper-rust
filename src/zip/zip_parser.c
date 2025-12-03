#ifdef _WIN32
#include <io.h>
#endif
#include "zip_parser.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#ifndef _WIN32
#include <sys/stat.h>
#endif

uint32_t read_u32_le(const uint8_t *data) {
  return data[0] | (data[1] << 8) | (data[2] << 16) | (data[3] << 24);
}

uint16_t read_u16_le(const uint8_t *data) { return data[0] | (data[1] << 8); }

uint64_t read_u64_le(const uint8_t *data) {
  return (uint64_t)data[0] | ((uint64_t)data[1] << 8) |
         ((uint64_t)data[2] << 16) | ((uint64_t)data[3] << 24) |
         ((uint64_t)data[4] << 32) | ((uint64_t)data[5] << 40) |
         ((uint64_t)data[6] << 48) | ((uint64_t)data[7] << 56);
}

int reader_init_file(reader_t *reader, const char *path) {
  FILE *file = fopen(path, "rb");
  if (!file) {
    return -1;
  }

#ifdef _WIN32
  if (_fseeki64(file, 0, SEEK_END) != 0) {
    fclose(file);
    return -1;
  }
  __int64 file_pos = _ftelli64(file);
  uint64_t size = (file_pos < 0) ? 0 : (uint64_t)file_pos;
  if (_fseeki64(file, 0, SEEK_SET) != 0) {
    fclose(file);
    return -1;
  }
#else
  fseek(file, 0, SEEK_END);
  long file_pos = ftell(file);
  uint64_t size = (file_pos < 0) ? 0 : (uint64_t)file_pos;
  fseek(file, 0, SEEK_SET);
#endif

  reader->type = READER_FILE;
  reader->data.file = file;
  reader->size = size;
  return 0;
}

#ifdef ENABLE_HTTP_SUPPORT
int reader_init_http(reader_t *reader, const char *url, const char *user_agent,
                     int silent) {
  reader->type = READER_HTTP;
  if (http_reader_init(&reader->data.http, url, silent) != 0) {
    return -1;
  }
  if (user_agent) {
    http_reader_set_user_agent(&reader->data.http, user_agent);
  }
  reader->size = reader->data.http.content_length;
  return 0;
}
#endif

void reader_cleanup(reader_t *reader) {
  if (reader->type == READER_FILE && reader->data.file) {
    fclose(reader->data.file);
    reader->data.file = NULL;
  }
#ifdef ENABLE_HTTP_SUPPORT
  else if (reader->type == READER_HTTP) {
    http_reader_cleanup(&reader->data.http);
  }
#endif
}

int reader_seek(reader_t *reader, uint64_t offset) {
  if (reader->type == READER_FILE) {
#ifdef _WIN32
    return _fseeki64(reader->data.file, (__int64)offset, SEEK_SET);
#else
    return fseek(reader->data.file, (long)offset, SEEK_SET);
#endif
  }
#ifdef ENABLE_HTTP_SUPPORT
  else if (reader->type == READER_HTTP) {
    return http_reader_seek(&reader->data.http, offset);
  }
#endif
  return -1;
}

int reader_read(reader_t *reader, uint8_t *buffer, size_t size,
                size_t *bytes_read) {
  if (reader->type == READER_FILE) {
    *bytes_read = fread(buffer, 1, size, reader->data.file);
    return (*bytes_read > 0 || feof(reader->data.file)) ? 0 : -1;
  }
#ifdef ENABLE_HTTP_SUPPORT
  else if (reader->type == READER_HTTP) {
    return http_reader_read(&reader->data.http, buffer, size, bytes_read);
  }
#endif
  return -1;
}

int reader_read_at(reader_t *reader, uint64_t offset, uint8_t *buffer,
                   size_t size, size_t *bytes_read) {
  if (reader->type == READER_FILE) {
#ifdef _WIN32
    if (_fseeki64(reader->data.file, (__int64)offset, SEEK_SET) != 0) {
      return -1;
    }
#else
    if (fseek(reader->data.file, (long)offset, SEEK_SET) != 0) {
      return -1;
    }
#endif
    *bytes_read = fread(buffer, 1, size, reader->data.file);
    return (*bytes_read > 0 || feof(reader->data.file)) ? 0 : -1;
  }
#ifdef ENABLE_HTTP_SUPPORT
  else if (reader->type == READER_HTTP) {
    return http_reader_read_at(&reader->data.http, offset, buffer, size,
                               bytes_read);
  }
#endif
  return -1;
}

uint64_t reader_get_size(reader_t *reader) { return reader->size; }

int find_eocd(reader_t *reader, uint64_t *eocd_offset, uint16_t *num_entries) {
  uint64_t file_size = reader_get_size(reader);
  uint64_t max_comment_size = 65535;
  uint64_t eocd_min_size = 22;

  uint64_t max_search;
  if (file_size <= max_comment_size + eocd_min_size) {
    max_search = file_size;
  } else {
    max_search = max_comment_size + eocd_min_size;
  }

  uint64_t chunk_size = 8192;
  uint64_t current_pos = file_size;
  uint8_t *buffer = malloc(chunk_size);
  if (!buffer) {
    return -1;
  }

  while (current_pos > 0) {
    uint64_t search_limit =
        (file_size >= max_search) ? (file_size - max_search) : 0;

    if (current_pos <= search_limit) {
      break;
    }

    uint64_t available_bytes = current_pos - search_limit;
    uint64_t read_size =
        (chunk_size < available_bytes) ? chunk_size : available_bytes;
    uint64_t read_pos = current_pos - read_size;

    size_t bytes_read;
    if (reader_read_at(reader, read_pos, buffer, read_size, &bytes_read) != 0 ||
        bytes_read == 0) {
      break;
    }

    for (size_t i = bytes_read; i >= 4; i--) {
      uint32_t sig = read_u32_le(&buffer[i - 4]);
      if (sig == EOCD_SIG) {
        *eocd_offset = read_pos + i - 4;
        if (i + 6 <= bytes_read) {
          *num_entries = read_u16_le(&buffer[i + 6]);
        } else {
          uint8_t num_entries_buf[2];
          size_t read_bytes;
          if (reader_read_at(reader, *eocd_offset + 10, num_entries_buf, 2,
                             &read_bytes) == 0) {
            *num_entries = read_u16_le(num_entries_buf);
          }
        }

        free(buffer);
        return 0;
      }
    }

    current_pos = read_pos;
    if (current_pos >= 3) {
      current_pos -= 3;
    }
  }

  free(buffer);
  return -1;
}

int read_zip64_eocd(reader_t *reader, uint64_t eocd_offset, uint64_t *cd_offset,
                    uint64_t *num_entries) {
  if (eocd_offset < 20) {
    return -1;
  }

  uint64_t search_start = eocd_offset - 20;
  uint64_t search_size = 20;

  uint8_t *buffer = malloc(search_size);
  if (!buffer) {
    return -1;
  }

  size_t bytes_read;
  if (reader_read_at(reader, search_start, buffer, search_size, &bytes_read) !=
      0) {
    free(buffer);
    return -1;
  }

  uint64_t zip64_eocd_offset = 0;
  int found_locator = 0;

  for (size_t i = bytes_read; i >= 4; i--) {
    uint32_t sig = read_u32_le(&buffer[i - 4]);
    if (sig == ZIP64_EOCD_LOCATOR_SIG) {
      found_locator = 1;
      if (i + 12 <= bytes_read) {
        zip64_eocd_offset = read_u64_le(&buffer[i + 4]);
      }
      break;
    }
  }

  free(buffer);

  if (!found_locator) {
    return -1;
  }

  uint8_t zip64_eocd[56];
  if (reader_read_at(reader, zip64_eocd_offset, zip64_eocd, 56, &bytes_read) !=
          0 ||
      bytes_read < 56) {
    return -1;
  }

  uint32_t sig = read_u32_le(zip64_eocd);
  if (sig != ZIP64_EOCD_SIG) {
    return -1;
  }

  *cd_offset = read_u64_le(&zip64_eocd[48]);
  *num_entries = read_u64_le(&zip64_eocd[32]);

  return 0;
}

int get_central_directory_info(reader_t *reader, uint64_t *cd_offset,
                               uint64_t *num_entries) {
  uint64_t eocd_offset;
  uint16_t num_entries_16;

  if (find_eocd(reader, &eocd_offset, &num_entries_16) != 0) {
    return -1;
  }

  uint8_t cd_offset_buf[4];
  size_t bytes_read;
  if (reader_read_at(reader, eocd_offset + 16, cd_offset_buf, 4, &bytes_read) !=
      0) {
    return -1;
  }

  uint32_t cd_offset_32 = read_u32_le(cd_offset_buf);

  if (cd_offset_32 == 0xFFFFFFFF) {
    return read_zip64_eocd(reader, eocd_offset, cd_offset, num_entries);
  } else {
    *cd_offset = cd_offset_32;
    *num_entries = num_entries_16;
    return 0;
  }
}

int read_central_directory_entry(reader_t *reader, zip_entry_t *entry) {
  uint8_t entry_header[46];
  size_t bytes_read;

  if (reader_read(reader, entry_header, 46, &bytes_read) != 0 ||
      bytes_read < 46) {
    return -1;
  }

  uint32_t sig = read_u32_le(entry_header);
  if (sig != CENTRAL_DIR_HEADER_SIG) {
    return -1;
  }

  entry->compression_method = read_u16_le(&entry_header[10]);
  uint16_t filename_len = read_u16_le(&entry_header[28]);
  uint16_t extra_len = read_u16_le(&entry_header[30]);
  uint16_t comment_len = read_u16_le(&entry_header[32]);

  uint64_t local_header_offset = read_u32_le(&entry_header[42]);
  uint64_t compressed_size = read_u32_le(&entry_header[20]);
  uint64_t uncompressed_size = read_u32_le(&entry_header[24]);

  if (filename_len >= sizeof(entry->name)) {
    filename_len = sizeof(entry->name) - 1;
  }

  if (reader_read(reader, (uint8_t *)entry->name, filename_len, &bytes_read) !=
      0) {
    return -1;
  }
  entry->name[filename_len] = '\0';

  uint8_t *extra_data = NULL;
  if (extra_len > 0) {
    extra_data = malloc(extra_len);
    if (extra_data &&
        reader_read(reader, extra_data, extra_len, &bytes_read) == 0) {
      if (local_header_offset == 0xFFFFFFFF || compressed_size == 0xFFFFFFFF ||
          uncompressed_size == 0xFFFFFFFF) {
        uint32_t pos = 0;
        while (pos + 4 <= extra_len) {
          uint16_t header_id = read_u16_le(&extra_data[pos]);
          uint16_t data_size = read_u16_le(&extra_data[pos + 2]);

          if (header_id == 0x0001 &&
              (uint32_t)(pos + 4 + data_size) <= extra_len) {
            uint32_t field_pos = pos + 4;
            uint32_t section_end = pos + 4 + data_size;

            if (uncompressed_size == 0xFFFFFFFF &&
                field_pos + 8 <= section_end) {
              uncompressed_size = read_u64_le(&extra_data[field_pos]);
              field_pos += 8;
            }

            if (compressed_size == 0xFFFFFFFF && field_pos + 8 <= section_end) {
              compressed_size = read_u64_le(&extra_data[field_pos]);
              field_pos += 8;
            }

            if (local_header_offset == 0xFFFFFFFF &&
                field_pos + 8 <= section_end) {
              local_header_offset = read_u64_le(&extra_data[field_pos]);
            }
            break;
          }

          if ((uint32_t)(4 + data_size) > UINT32_MAX - pos)
            break; // Prevent overflow
          pos += (uint32_t)(4 + data_size);
        }
      }
    }
    free(extra_data);
  }

  // Skip comment
  if (comment_len > 0) {
    if (reader->type == READER_FILE) {
      fseek(reader->data.file, comment_len, SEEK_CUR);
    }
#ifdef ENABLE_HTTP_SUPPORT
    else if (reader->type == READER_HTTP) {
      reader->data.http.current_pos += comment_len;
    }
#endif
  }

  entry->compressed_size = compressed_size;
  entry->uncompressed_size = uncompressed_size;
  entry->local_header_offset = local_header_offset;
  entry->data_offset = 0;

  return 0;
}

int find_payload_entry(reader_t *reader, zip_entry_t *payload_entry) {
  uint64_t cd_offset, num_entries;

  if (get_central_directory_info(reader, &cd_offset, &num_entries) != 0) {
    return -1;
  }

  if (reader_seek(reader, cd_offset) != 0) {
    return -1;
  }

  for (uint64_t i = 0; i < num_entries; i++) {
    zip_entry_t entry;
    if (read_central_directory_entry(reader, &entry) != 0) {
      continue;
    }

    if (entry.compression_method != 0) {
      continue;
    }

    if (strcmp(entry.name, "payload.bin") == 0 ||
        strstr(entry.name, "/payload.bin") != NULL) {
      *payload_entry = entry;
      return 0;
    }
  }

  return -1;
}

int get_data_offset(reader_t *reader, zip_entry_t *entry) {
  uint8_t local_header[30];
  size_t bytes_read;

  if (reader_read_at(reader, entry->local_header_offset, local_header, 30,
                     &bytes_read) != 0 ||
      bytes_read < 30) {
    return -1;
  }

  uint32_t sig = read_u32_le(local_header);
  if (sig != LOCAL_FILE_HEADER_SIG) {
    return -1;
  }

  uint16_t local_compression = read_u16_le(&local_header[8]);
  if (local_compression != 0) {
    return -1;
  }

  uint16_t local_filename_len = read_u16_le(&local_header[26]);
  uint16_t local_extra_len = read_u16_le(&local_header[28]);

  entry->data_offset =
      entry->local_header_offset + 30 + local_filename_len + local_extra_len;
  return 0;
}

int verify_payload_magic(reader_t *reader, uint64_t offset) {
  uint8_t magic[4];
  size_t bytes_read;

  if (reader_read_at(reader, offset, magic, 4, &bytes_read) != 0 ||
      bytes_read < 4) {
    return -1;
  }

  if (memcmp(magic, "CrAU", 4) != 0) {
    return -1;
  }

  return 0;
}