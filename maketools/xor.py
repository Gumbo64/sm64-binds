# xor.py
import sys


def xor_bytes(data: bytes, key: bytes) -> bytes:
    """XOR data with the key, repeating the key as needed (runs at C speed)."""
    key_len = len(key)
    repeated_key = (key * (len(data) // key_len + 1))[:len(data)]
    result = int.from_bytes(data, "little") ^ int.from_bytes(repeated_key, "little")
    return result.to_bytes(len(data), "little")


def xor_files(input_file, output_file, key_file):
    with open(key_file, 'rb') as kf:
        key = kf.read()

    with open(input_file, 'rb') as f1:
        data = f1.read()

    with open(output_file, 'wb') as f2:
        f2.write(xor_bytes(data, key))


if __name__ == '__main__':
    if len(sys.argv) != 4:
        print("Usage: {} <input_file> <output_file> <key_file>".format(sys.argv[0]))
        sys.exit(1)

    input_file = sys.argv[1]
    output_file = sys.argv[2]
    key_file = sys.argv[3]
    xor_files(input_file, output_file, key_file)
