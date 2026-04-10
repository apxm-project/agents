"""Setup script for apxm_vllm package."""

from setuptools import setup, find_packages

setup(
    name="apxm_vllm",
    version="0.1.0",
    description="APXM vLLM Graph-Aware Scheduler Extension",
    author="APXM Contributors",
    packages=find_packages(),
    python_requires=">=3.9",
    install_requires=[
        "fastapi>=0.104.0",
        "uvicorn[standard]>=0.24.0",
        "pydantic>=2.0.0",
    ],
    extras_require={
        "test": [
            "pytest>=7.4.0",
            "pytest-asyncio>=0.21.0",
            "httpx>=0.24.0",
        ],
    },
    entry_points={
        "console_scripts": [
            "apxm-vllm-server=apxm_vllm.api:run_server",
        ],
    },
)
