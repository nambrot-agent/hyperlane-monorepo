// SPDX-License-Identifier: MIT OR Apache-2.0
pragma solidity >=0.8.0;

import {MerkleLib} from "../libs/Merkle.sol";
import {MerkleTreeHook} from "../hooks/MerkleTreeHook.sol";

contract TestMerkleTreeHook is MerkleTreeHook {
    using MerkleLib for MerkleLib.Tree;

    // Store all inserted messages for proof generation
    bytes32[] private messages;

    constructor(address _mailbox) MerkleTreeHook(_mailbox) {}

    function proof() external view returns (bytes32[32] memory) {
        bytes32[32] memory _zeroes = MerkleLib.zeroHashes();
        uint256 _index = _tree.count - 1;
        bytes32[32] memory _proof;

        for (uint256 i = 0; i < 32; i++) {
            uint256 _ithBit = (_index >> i) & 0x01;
            if (_ithBit == 1) {
                _proof[i] = _tree.branch[i];
            } else {
                _proof[i] = _zeroes[i];
            }
        }
        return _proof;
    }

    function proofFor(uint256 _leafIndex) external view returns (bytes32[32] memory) {
        require(_leafIndex < messages.length, "Index out of bounds");
        bytes32[32] memory _proof;
        bytes32[32] memory _zeroes = MerkleLib.zeroHashes();

        // Build proof by determining sibling at each level
        uint256 index = _leafIndex;
        for (uint256 level = 0; level < 32; level++) {
            // Find the sibling index at the current level
            uint256 siblingIndex = index ^ 1;

            // Compute the range of leaves covered by the sibling subtree
            uint256 siblingStart = siblingIndex * (1 << level);
            uint256 subtreeSize = 1 << level;

            // Compute the hash of the sibling subtree
            _proof[level] = _computeRangeHash(siblingStart, siblingStart + subtreeSize, level, _zeroes);

            // Move up to parent level
            index = index >> 1;
        }

        return _proof;
    }

    function _computeRangeHash(
        uint256 _startLeaf,
        uint256 _endLeaf,
        uint256 _level,
        bytes32[32] memory _zeroes
    ) private view returns (bytes32) {
        // If the entire range is beyond our messages, return zero hash
        if (_startLeaf >= messages.length) {
            return _zeroes[_level];
        }

        // Base case: single leaf
        if (_level == 0) {
            return messages[_startLeaf];
        }

        // Split into left and right subtrees
        uint256 mid = _startLeaf + (1 << (_level - 1));
        bytes32 left = _computeRangeHash(_startLeaf, mid, _level - 1, _zeroes);
        bytes32 right = _computeRangeHash(mid, _endLeaf, _level - 1, _zeroes);

        return keccak256(abi.encodePacked(left, right));
    }

    function insert(bytes32 _id) external {
        messages.push(_id);
        _tree.insert(_id);
    }
}
