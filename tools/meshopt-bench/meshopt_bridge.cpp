#define NOMINMAX
#include <windows.h>
#include <psapi.h>

#include <algorithm>
#include <chrono>
#include <cstdint>
#include <fstream>
#include <iostream>
#include <stdexcept>
#include <string>
#include <vector>

#include "meshoptimizer.h"

namespace
{

using Clock = std::chrono::steady_clock;

std::vector<unsigned char> readFile(const char* path)
{
	std::ifstream file(path, std::ios::binary);
	if (!file)
		throw std::runtime_error(std::string("can't open input: ") + path);
	file.seekg(0, std::ios::end);
	const std::streamoff size = file.tellg();
	if (size < 0)
		throw std::runtime_error(std::string("can't size input: ") + path);
	file.seekg(0, std::ios::beg);
	std::vector<unsigned char> bytes(static_cast<size_t>(size));
	if (!bytes.empty() && !file.read(reinterpret_cast<char*>(bytes.data()), size))
		throw std::runtime_error(std::string("can't read input: ") + path);
	return bytes;
}

void writeFile(const char* path, const std::vector<unsigned char>& bytes)
{
	std::ofstream file(path, std::ios::binary | std::ios::trunc);
	if (!file)
		throw std::runtime_error(std::string("can't open output: ") + path);
	if (!bytes.empty())
		file.write(reinterpret_cast<const char*>(bytes.data()), static_cast<std::streamsize>(bytes.size()));
	if (!file)
		throw std::runtime_error(std::string("can't write output: ") + path);
}

double milliseconds(Clock::time_point start, Clock::time_point end)
{
	return std::chrono::duration<double, std::milli>(end - start).count();
}

double median(std::vector<double> values)
{
	std::sort(values.begin(), values.end());
	return values[values.size() / 2];
}

size_t peakWorkingSet()
{
	PROCESS_MEMORY_COUNTERS_EX counters = {};
	counters.cb = sizeof(counters);
	return GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&counters), sizeof(counters))
		? static_cast<size_t>(counters.PeakWorkingSetSize)
		: 0;
}

} // namespace

int main(int argc, char** argv)
{
	try
	{
		if (argc != 9)
		{
			std::cerr << "usage: meshopt-bridge <input> <positions-f32x3> <count> <stride> <prepared> <encoded> <decoded> <level>\n";
			return 2;
		}

		const size_t count = static_cast<size_t>(std::stoull(argv[3]));
		const size_t stride = static_cast<size_t>(std::stoull(argv[4]));
		const int level = std::stoi(argv[8]);
		if (count == 0 || stride == 0 || stride > 256 || stride % 4 != 0 || level < 0 || level > 3)
			throw std::runtime_error("count, stride (multiple of four, at most 256), or level is invalid");
		if (count > SIZE_MAX / stride || count > SIZE_MAX / 12)
			throw std::runtime_error("input size overflows size_t");

		const std::vector<unsigned char> input = readFile(argv[1]);
		const std::vector<unsigned char> positionsBytes = readFile(argv[2]);
		if (input.size() != count * stride)
			throw std::runtime_error("input size does not match count * stride");
		if (positionsBytes.size() != count * 12)
			throw std::runtime_error("position file size does not match count * 12");

		std::vector<unsigned int> oldToNew(count);
		std::vector<unsigned char> prepared(input.size());
		const auto prepareStart = Clock::now();
		meshopt_spatialSortRemap(oldToNew.data(), reinterpret_cast<const float*>(positionsBytes.data()), count, 12);
		for (size_t oldIndex = 0; oldIndex < count; ++oldIndex)
			std::copy_n(input.data() + oldIndex * stride, stride, prepared.data() + static_cast<size_t>(oldToNew[oldIndex]) * stride);
		const double prepareMs = milliseconds(prepareStart, Clock::now());
		writeFile(argv[5], prepared);

		const size_t bound = meshopt_encodeVertexBufferBound(count, stride);
		std::vector<unsigned char> encoded(bound);
		std::vector<double> encodeMs;
		for (int repeat = 0; repeat < 3; ++repeat)
		{
			const auto start = Clock::now();
			const size_t written = meshopt_encodeVertexBufferLevel(encoded.data(), encoded.size(), prepared.data(), count, stride, level, 1);
			encodeMs.push_back(milliseconds(start, Clock::now()));
			if (written == 0)
				throw std::runtime_error("meshopt vertex encoder failed");
			encoded.resize(written);
			if (repeat != 2)
				encoded.resize(bound);
		}
		writeFile(argv[6], encoded);

		std::vector<unsigned char> decoded(prepared.size());
		std::vector<double> decodeMs;
		for (int repeat = 0; repeat < 3; ++repeat)
		{
			const auto start = Clock::now();
			const int status = meshopt_decodeVertexBuffer(decoded.data(), count, stride, encoded.data(), encoded.size());
			decodeMs.push_back(milliseconds(start, Clock::now()));
			if (status != 0)
				throw std::runtime_error("meshopt vertex decoder failed with status " + std::to_string(status));
		}
		const bool exact = prepared == decoded;
		if (!exact)
			throw std::runtime_error("meshopt decoder did not reproduce the prepared input exactly");
		writeFile(argv[7], decoded);

		std::cout << "{\"meshopt_version\":" << MESHOPTIMIZER_VERSION
			<< ",\"vertex_count\":" << count
			<< ",\"stride\":" << stride
			<< ",\"prepared_bytes\":" << prepared.size()
			<< ",\"encoded_bytes\":" << encoded.size()
			<< ",\"prepare_ms\":" << prepareMs
			<< ",\"encode_ms_median\":" << median(encodeMs)
			<< ",\"decode_ms_median\":" << median(decodeMs)
			<< ",\"roundtrip_exact\":" << (exact ? "true" : "false")
			<< ",\"peak_working_set_bytes\":" << peakWorkingSet()
			<< "}\n";
		return 0;
	}
	catch (const std::exception& error)
	{
		std::cerr << error.what() << '\n';
		return 1;
	}
}
